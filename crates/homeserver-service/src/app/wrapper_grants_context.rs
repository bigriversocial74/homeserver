fn connection_context(connection: &Connection, connection_id: &str) -> Result<ConnectionContext> {
    let connection_id = validate_uuid(connection_id, "connection ID")?;
    connection
        .query_row(
            "SELECT c.wrapper_id,c.connection_id,c.grant_revision FROM wrapper_connections c JOIN wrapper_identities w ON w.wrapper_id=c.wrapper_id WHERE c.connection_id=?1 AND c.lifecycle_state IN ('active','offline','grace') AND w.state='active'",
            params![connection_id],
            |row| {
                let revision: i64 = row.get(2)?;
                Ok(ConnectionContext {
                    wrapper_id: row.get(0)?,
                    connection_id: row.get(1)?,
                    grant_revision: revision.max(0) as u64,
                })
            },
        )
        .context("active wrapper connection was not found")
}

fn capability_rule(connection: &Connection, capability_key: &str) -> Result<CapabilityRule> {
    connection
        .query_row(
            "SELECT risk_tier,default_approval_mode,result_mode,requires_scope,allowed_operations_json FROM wrapper_capability_catalog WHERE capability_key=?1 AND state='active'",
            params![capability_key],
            |row| {
                let operations_json: String = row.get(4)?;
                Ok(CapabilityRule {
                    risk_tier: row.get(0)?,
                    default_approval_mode: row.get(1)?,
                    result_mode: row.get(2)?,
                    requires_scope: row.get::<_, i64>(3)? == 1,
                    allowed_operations: serde_json::from_str(&operations_json).unwrap_or_default(),
                })
            },
        )
        .context("capability is not available")
}

fn active_grant(
    connection: &Connection,
    connection_id: &str,
    capability_key: &str,
) -> Result<Option<StoredGrant>> {
    connection
        .query_row(
            "SELECT grant_id,wrapper_id,connection_id,capability_key,grant_revision,allowed_operations_json,approval_mode,state,expires_at_utc,supersedes_grant_id FROM wrapper_capability_grants WHERE connection_id=?1 AND capability_key=?2 AND state='active' AND not_before_utc<=?3 AND expires_at_utc>?3 ORDER BY grant_revision DESC LIMIT 1",
            params![connection_id, capability_key, now_utc()],
            stored_grant_from_row,
        )
        .optional()
        .map_err(Into::into)
}

