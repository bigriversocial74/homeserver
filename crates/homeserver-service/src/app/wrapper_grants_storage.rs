fn stored_grant(connection: &Connection, grant_id: &str) -> Result<StoredGrant> {
    let grant_id = validate_uuid(grant_id, "grant ID")?;
    connection
        .query_row(
            "SELECT grant_id,wrapper_id,connection_id,capability_key,grant_revision,allowed_operations_json,approval_mode,state,expires_at_utc,supersedes_grant_id FROM wrapper_capability_grants WHERE grant_id=?1",
            params![grant_id],
            stored_grant_from_row,
        )
        .context("grant was not found")
}

fn stored_grant_tx(transaction: &Transaction<'_>, grant_id: &str) -> Result<StoredGrant> {
    transaction
        .query_row(
            "SELECT grant_id,wrapper_id,connection_id,capability_key,grant_revision,allowed_operations_json,approval_mode,state,expires_at_utc,supersedes_grant_id FROM wrapper_capability_grants WHERE grant_id=?1",
            params![grant_id],
            stored_grant_from_row,
        )
        .context("grant was not found")
}

fn stored_grant_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredGrant> {
    let operations_json: String = row.get(5)?;
    let revision: i64 = row.get(4)?;
    Ok(StoredGrant {
        grant_id: row.get(0)?,
        wrapper_id: row.get(1)?,
        connection_id: row.get(2)?,
        capability_key: row.get(3)?,
        grant_revision: revision.max(0) as u64,
        allowed_operations: serde_json::from_str(&operations_json).unwrap_or_default(),
        approval_mode: row.get(6)?,
        state: row.get(7)?,
        expires_at_utc: row.get(8)?,
        supersedes_grant_id: row.get(9)?,
    })
}

fn stored_bridge(connection: &Connection, bridge_id: &str) -> Result<BridgeGrant> {
    let bridge_id = validate_uuid(bridge_id, "bridge ID")?;
    connection
        .query_row(
            "SELECT bridge_id,source_wrapper_id,source_connection_id,target_wrapper_id,target_connection_id,capability_key,allowed_operations_json,scope_kind,scope_value,result_policy,approval_mode,state,issued_by_user_id,reason,request_hash,not_before_utc,expires_at_utc,approved_by_user_id,approved_at_utc,revoked_at_utc FROM wrapper_bridge_grants WHERE bridge_id=?1",
            params![bridge_id],
            bridge_from_row,
        )
        .context("bridge was not found")
}

fn stored_bridge_tx(transaction: &Transaction<'_>, bridge_id: &str) -> Result<BridgeGrant> {
    transaction
        .query_row(
            "SELECT bridge_id,source_wrapper_id,source_connection_id,target_wrapper_id,target_connection_id,capability_key,allowed_operations_json,scope_kind,scope_value,result_policy,approval_mode,state,issued_by_user_id,reason,request_hash,not_before_utc,expires_at_utc,approved_by_user_id,approved_at_utc,revoked_at_utc FROM wrapper_bridge_grants WHERE bridge_id=?1",
            params![bridge_id],
            bridge_from_row,
        )
        .context("bridge was not found")
}

fn bridge_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BridgeGrant> {
    let operations_json: String = row.get(6)?;
    Ok(BridgeGrant {
        bridge_id: row.get(0)?,
        source_wrapper_id: row.get(1)?,
        source_connection_id: row.get(2)?,
        target_wrapper_id: row.get(3)?,
        target_connection_id: row.get(4)?,
        capability_key: row.get(5)?,
        allowed_operations: serde_json::from_str(&operations_json).unwrap_or_default(),
        scope_kind: row.get(7)?,
        scope_value: row.get(8)?,
        result_policy: row.get(9)?,
        approval_mode: row.get(10)?,
        state: row.get(11)?,
        issued_by_user_id: row.get(12)?,
        reason: row.get(13)?,
        request_hash: row.get(14)?,
        not_before_utc: row.get(15)?,
        expires_at_utc: row.get(16)?,
        approved_by_user_id: row.get(17)?,
        approved_at_utc: row.get(18)?,
        revoked_at_utc: row.get(19)?,
    })
}

fn matching_scope(
    connection: &Connection,
    grant_id: &str,
    scope_kind: &str,
    scope_value: &str,
) -> Result<Option<String>> {
    let kind = validate_scope_kind(scope_kind)?;
    let value = validate_scope_value(scope_value)?;
    connection
        .query_row(
            "SELECT result_policy FROM wrapper_dataset_scopes WHERE grant_id=?1 AND scope_kind=?2 AND scope_value=?3 AND state='active'",
            params![grant_id, kind, value],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

fn consume_use_approval(
    transaction: &Transaction<'_>,
    grant_id: &str,
    approval_id: &str,
    plan_hash: &str,
) -> Result<()> {
    let now = now_utc();
    let count = transaction.execute(
        "UPDATE wrapper_grant_approvals SET state='consumed',consumed_at_utc=?1 WHERE approval_id=?2 AND grant_id=?3 AND approval_action='sensitive_use' AND plan_hash=?4 AND state='approved' AND expires_at_utc>?1",
        params![now, approval_id, grant_id, plan_hash],
    )?;
    ensure!(count == 1, "valid sensitive-use approval was not found");
    Ok(())
}

