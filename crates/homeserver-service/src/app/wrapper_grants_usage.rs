fn enforce_and_record_usage(
    transaction: &Transaction<'_>,
    grant_id: &str,
    result_bytes: u64,
    token_count: u64,
) -> Result<()> {
    let limits = read_limits_tx(transaction, grant_id)?;
    ensure!(
        result_bytes <= limits.max_result_bytes,
        "authorized result exceeds the grant result-size limit"
    );
    let now = Utc::now();
    let minute_start = now.format("%Y-%m-%dT%H:%M:00.000Z").to_string();
    let day_start = now.format("%Y-%m-%dT00:00:00.000Z").to_string();
    let minute_requests: i64 = transaction
        .query_row(
            "SELECT request_count FROM wrapper_grant_usage_windows WHERE grant_id=?1 AND window_kind='minute' AND window_start_utc=?2",
            params![grant_id, minute_start],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    ensure!(
        minute_requests < i64::from(limits.requests_per_minute),
        "grant request rate limit exceeded"
    );
    let daily_tokens: i64 = transaction
        .query_row(
            "SELECT token_count FROM wrapper_grant_usage_windows WHERE grant_id=?1 AND window_kind='day' AND window_start_utc=?2",
            params![grant_id, day_start],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    ensure!(
        daily_tokens.saturating_add(token_count as i64) <= limits.max_daily_tokens as i64,
        "grant daily token limit exceeded"
    );
    let updated = now_utc();
    transaction.execute(
        "INSERT INTO wrapper_grant_usage_windows (grant_id,window_kind,window_start_utc,request_count,result_bytes,token_count,updated_at_utc) VALUES (?1,'minute',?2,1,?3,?4,?5) ON CONFLICT(grant_id,window_kind,window_start_utc) DO UPDATE SET request_count=request_count+1,result_bytes=result_bytes+excluded.result_bytes,token_count=token_count+excluded.token_count,updated_at_utc=excluded.updated_at_utc",
        params![grant_id,minute_start,result_bytes as i64,token_count as i64,updated],
    )?;
    transaction.execute(
        "INSERT INTO wrapper_grant_usage_windows (grant_id,window_kind,window_start_utc,request_count,result_bytes,token_count,updated_at_utc) VALUES (?1,'day',?2,1,?3,?4,?5) ON CONFLICT(grant_id,window_kind,window_start_utc) DO UPDATE SET request_count=request_count+1,result_bytes=result_bytes+excluded.result_bytes,token_count=token_count+excluded.token_count,updated_at_utc=excluded.updated_at_utc",
        params![grant_id,day_start,result_bytes as i64,token_count as i64,updated],
    )?;
    Ok(())
}

fn activate_supersession(
    transaction: &Transaction<'_>,
    new_grant_id: &str,
    supersedes_grant_id: Option<&str>,
    now: &str,
) -> Result<()> {
    if let Some(old_grant_id) = supersedes_grant_id {
        transaction.execute(
            "UPDATE wrapper_capability_grants SET state='superseded',superseded_by_grant_id=?1,updated_at_utc=?2 WHERE grant_id=?3 AND state IN ('active','pending_approval')",
            params![new_grant_id, now, old_grant_id],
        )?;
        transaction.execute(
            "UPDATE wrapper_dataset_scopes SET state='revoked',revoked_at_utc=?1 WHERE grant_id=?2 AND state='active'",
            params![now, old_grant_id],
        )?;
    }
    Ok(())
}

fn advance_authority_revision(
    transaction: &Transaction<'_>,
    connection_id: &str,
    reason: &str,
    now: &str,
) -> Result<u64> {
    transaction.execute(
        "UPDATE wrapper_connections SET grant_revision=grant_revision+1,updated_at_utc=?1 WHERE connection_id=?2",
        params![now, connection_id],
    )?;
    let revision: i64 = transaction.query_row(
        "SELECT grant_revision FROM wrapper_connections WHERE connection_id=?1",
        params![connection_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO wrapper_grant_revocation_fences (connection_id,grant_revision,reason,updated_at_utc) VALUES (?1,?2,?3,?4) ON CONFLICT(connection_id) DO UPDATE SET grant_revision=excluded.grant_revision,reason=excluded.reason,updated_at_utc=excluded.updated_at_utc",
        params![connection_id, revision, reason, now],
    )?;
    Ok(revision.max(0) as u64)
}

#[allow(clippy::too_many_arguments)]
fn record_event(
    transaction: &Transaction<'_>,
    wrapper_id: &str,
    connection_id: &str,
    grant_id: Option<&str>,
    bridge_id: Option<&str>,
    event_type: &str,
    outcome: &str,
    actor_user_id: Option<&str>,
    detail_code: &str,
    metadata: Value,
    created_at: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO wrapper_grant_events (event_id,wrapper_id,connection_id,grant_id,bridge_id,event_type,outcome,actor_user_id,detail_code,metadata_json,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            Uuid::new_v4().to_string(),
            wrapper_id,
            connection_id,
            grant_id,
            bridge_id,
            event_type,
            outcome,
            actor_user_id,
            detail_code,
            serde_json::to_string(&metadata)?,
            created_at
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_approval(
    transaction: &Transaction<'_>,
    grant_id: Option<&str>,
    bridge_id: Option<&str>,
    action: &str,
    plan_hash: &str,
    requested_by: &str,
    expires_at: &str,
    created_at: &str,
    reason: Option<&str>,
) -> Result<String> {
    let approval_id = Uuid::new_v4().to_string();
    transaction.execute(
        "INSERT INTO wrapper_grant_approvals (approval_id,grant_id,bridge_id,approval_action,plan_hash,state,requested_by_user_id,expires_at_utc,created_at_utc,reason) VALUES (?1,?2,?3,?4,?5,'pending',?6,?7,?8,?9)",
        params![
            approval_id,
            grant_id,
            bridge_id,
            action,
            plan_hash,
            requested_by,
            expires_at,
            created_at,
            reason
        ],
    )?;
    Ok(approval_id)
}

fn insert_scopes(
    transaction: &Transaction<'_>,
    grant_id: &str,
    scopes: &[ScopeInput],
    created_at: &str,
) -> Result<()> {
    for scope in scopes {
        transaction.execute(
            "INSERT INTO wrapper_dataset_scopes (scope_id,grant_id,scope_kind,scope_value,allowed_fields_json,filter_json,result_policy,state,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,'active',?8)",
            params![
                Uuid::new_v4().to_string(),
                grant_id,
                scope.scope_kind,
                scope.scope_value,
                serde_json::to_string(&scope.allowed_fields)?,
                serde_json::to_string(&scope.filter)?,
                scope.result_policy.as_deref().unwrap_or("safe_result"),
                created_at
            ],
        )?;
    }
    Ok(())
}

fn insert_limits(
    transaction: &Transaction<'_>,
    grant_id: &str,
    limits: &ResourceLimits,
    created_at: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO wrapper_resource_limits (grant_id,requests_per_minute,max_result_bytes,max_daily_tokens,max_concurrent_jobs,max_queued_jobs,max_execution_seconds,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",
        params![
            grant_id,
            limits.requests_per_minute,
            limits.max_result_bytes as i64,
            limits.max_daily_tokens as i64,
            limits.max_concurrent_jobs,
            limits.max_queued_jobs,
            limits.max_execution_seconds,
            created_at
        ],
    )?;
    Ok(())
}

