fn create_grant(
    connection: &Connection,
    request: CreateGrantRequest,
    supersedes_grant_id: Option<String>,
) -> Result<String> {
    expire_stale_authority(connection)?;
    let context = connection_context(connection, &request.connection_id)?;
    ensure!(
        context.wrapper_id == request.wrapper_id,
        "connection does not belong to the requested wrapper"
    );
    let capability_key = validate_capability_key(&request.capability_key)?;
    let rule = capability_rule(connection, &capability_key)?;
    let operations = normalize_operations(request.allowed_operations)?;
    ensure_operation_subset(&operations, &rule.allowed_operations)?;
    let approval_mode = resolve_approval_mode(request.approval_mode.as_deref(), &rule)?;
    let scopes = normalize_scopes(request.scopes, &rule)?;
    let limits = normalize_limits(request.limits, &rule.risk_tier)?;
    let issued_by = bounded_text(&request.issued_by_user_id, 1, 160, "issuing user")?;
    let reason = bounded_text(&request.reason, 1, 500, "grant reason")?;
    let now = Utc::now();
    let expires_at = validated_expiration(now, request.expires_minutes, &rule.risk_tier)?;
    let grant_id = Uuid::new_v4().to_string();
    let next_revision: i64 = connection.query_row(
        "SELECT COALESCE(MAX(grant_revision),0)+1 FROM wrapper_capability_grants WHERE connection_id=?1 AND capability_key=?2",
        params![context.connection_id, capability_key],
        |row| row.get(0),
    )?;
    let request_hash = hash_json(&json!({
        "wrapper_id": &context.wrapper_id,
        "connection_id": &context.connection_id,
        "capability_key": &capability_key,
        "grant_revision": next_revision,
        "operations": &operations,
        "approval_mode": &approval_mode,
        "scopes": &scopes,
        "limits": &limits,
        "not_before_utc": timestamp(now),
        "expires_at_utc": timestamp(expires_at),
        "supersedes_grant_id": &supersedes_grant_id
    }))?;
    let state = if approval_mode == "none" {
        "active"
    } else {
        "pending_approval"
    };
    let insert_state = if state == "active" && supersedes_grant_id.is_some() {
        "pending_approval"
    } else {
        state
    };
    let now_text = timestamp(now);
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO wrapper_capability_grants (grant_id,wrapper_id,connection_id,capability_key,grant_revision,allowed_operations_json,approval_mode,state,issued_by_user_id,reason,request_hash,not_before_utc,expires_at_utc,approved_by_user_id,approved_at_utc,supersedes_grant_id,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,CASE WHEN ?8='active' THEN ?9 ELSE NULL END,CASE WHEN ?8='active' THEN ?12 ELSE NULL END,?14,?12,?12)",
        params![
            grant_id,
            context.wrapper_id,
            context.connection_id,
            capability_key,
            next_revision,
            serde_json::to_string(&operations)?,
            approval_mode,
            insert_state,
            issued_by,
            reason,
            request_hash,
            now_text,
            timestamp(expires_at),
            supersedes_grant_id
        ],
    )?;
    insert_scopes(&transaction, &grant_id, &scopes, &now_text)?;
    insert_limits(&transaction, &grant_id, &limits, &now_text)?;
    if state == "pending_approval" {
        insert_approval(
            &transaction,
            Some(&grant_id),
            None,
            if supersedes_grant_id.is_some() {
                "grant_rotate"
            } else {
                "grant_create"
            },
            &request_hash,
            &issued_by,
            &timestamp(now + Duration::minutes(30)),
            &now_text,
            Some("Grant requires explicit owner approval"),
        )?;
    } else {
        activate_supersession(&transaction, &grant_id, supersedes_grant_id.as_deref(), &now_text)?;
        if insert_state != "active" {
            transaction.execute(
                "UPDATE wrapper_capability_grants SET state='active',approved_by_user_id=?1,approved_at_utc=?2,updated_at_utc=?2 WHERE grant_id=?3",
                params![issued_by, now_text, grant_id],
            )?;
        }
        advance_authority_revision(
            &transaction,
            &context.connection_id,
            "grant activated",
            &now_text,
        )?;
    }
    record_event(
        &transaction,
        &context.wrapper_id,
        &context.connection_id,
        Some(&grant_id),
        None,
        "wrapper.grant.created",
        "success",
        Some(&issued_by),
        if state == "active" {
            "grant_active"
        } else {
            "approval_required"
        },
        json!({
            "capability_key": capability_key,
            "grant_revision": next_revision,
            "state": state,
            "expires_at_utc": timestamp(expires_at)
        }),
        &now_text,
    )?;
    transaction.commit()?;
    Ok(grant_id)
}

fn rotate_grant(connection: &Connection, request: RotateGrantRequest) -> Result<String> {
    let existing = stored_grant(connection, &request.grant_id)?;
    ensure!(
        existing.state == "active" || existing.state == "pending_approval",
        "only active or pending grants can be rotated"
    );
    let scopes = read_scope_inputs(connection, &existing.grant_id)?;
    let limits = read_limits(connection, &existing.grant_id)?;
    create_grant(
        connection,
        CreateGrantRequest {
            wrapper_id: existing.wrapper_id,
            connection_id: existing.connection_id,
            capability_key: existing.capability_key,
            allowed_operations: existing.allowed_operations,
            scopes,
            limits: Some(ResourceLimitsInput {
                requests_per_minute: Some(limits.requests_per_minute),
                max_result_bytes: Some(limits.max_result_bytes),
                max_daily_tokens: Some(limits.max_daily_tokens),
                max_concurrent_jobs: Some(limits.max_concurrent_jobs),
                max_queued_jobs: Some(limits.max_queued_jobs),
                max_execution_seconds: Some(limits.max_execution_seconds),
            }),
            approval_mode: Some(existing.approval_mode),
            issued_by_user_id: request.issued_by_user_id,
            reason: request.reason,
            expires_minutes: request.expires_minutes,
        },
        Some(existing.grant_id),
    )
}

fn revoke_grant(connection: &Connection, request: RevokeGrantRequest) -> Result<()> {
    ensure!(
        request.confirmation.trim() == "REVOKE GRANT",
        "grant revocation requires the exact confirmation 'REVOKE GRANT'"
    );
    let actor = bounded_text(&request.actor_user_id, 1, 160, "revoking user")?;
    let reason = bounded_text(&request.reason, 1, 500, "revocation reason")?;
    let grant = stored_grant(connection, &request.grant_id)?;
    ensure!(
        !matches!(grant.state.as_str(), "revoked" | "expired" | "superseded"),
        "grant is already inactive"
    );
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE wrapper_capability_grants SET state='revoked',revoked_by_user_id=?1,revoked_at_utc=?2,revocation_reason=?3,updated_at_utc=?2 WHERE grant_id=?4",
        params![actor, now, reason, grant.grant_id],
    )?;
    transaction.execute(
        "UPDATE wrapper_dataset_scopes SET state='revoked',revoked_at_utc=?1 WHERE grant_id=?2 AND state='active'",
        params![now, grant.grant_id],
    )?;
    transaction.execute(
        "UPDATE wrapper_grant_approvals SET state='revoked',decided_by_user_id=?1,decided_at_utc=?2,reason=?3 WHERE grant_id=?4 AND state IN ('pending','approved')",
        params![actor, now, reason, grant.grant_id],
    )?;
    advance_authority_revision(
        &transaction,
        &grant.connection_id,
        &format!("grant {} revoked: {}", grant.grant_id, reason),
        &now,
    )?;
    record_event(
        &transaction,
        &grant.wrapper_id,
        &grant.connection_id,
        Some(&grant.grant_id),
        None,
        "wrapper.grant.revoked",
        "success",
        Some(&actor),
        "revoked",
        json!({"capability_key": grant.capability_key, "reason": reason}),
        &now,
    )?;
    transaction.commit()?;
    Ok(())
}

fn request_use_approval(
    connection: &Connection,
    request: RequestUseApprovalRequest,
) -> Result<String> {
    let grant = stored_grant(connection, &request.grant_id)?;
    ensure!(grant.state == "active", "grant is not active");
    ensure!(
        grant.approval_mode == "per_request",
        "grant does not require per-request approval"
    );
    let requested_by =
        bounded_text(&request.requested_by_user_id, 1, 160, "requesting user")?;
    let plan_hash = validate_sha256(&request.plan_hash, "plan hash")?;
    let reason = bounded_text(&request.reason, 1, 500, "approval reason")?;
    ensure!(
        (1..=30).contains(&request.expires_minutes),
        "use approval must expire within 1 to 30 minutes"
    );
    let now = Utc::now();
    let approval_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO wrapper_grant_approvals (approval_id,grant_id,approval_action,plan_hash,state,requested_by_user_id,expires_at_utc,created_at_utc,reason) VALUES (?1,?2,'sensitive_use',?3,'pending',?4,?5,?6,?7)",
        params![
            approval_id,
            grant.grant_id,
            plan_hash,
            requested_by,
            timestamp(now + Duration::minutes(i64::from(request.expires_minutes))),
            timestamp(now),
            reason
        ],
    )?;
    Ok(approval_id)
}

