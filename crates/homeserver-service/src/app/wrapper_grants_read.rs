fn read_catalog(connection: &Connection) -> Result<Vec<CapabilityCatalogEntry>> {
    let mut statement = connection.prepare(
        "SELECT capability_key,description,risk_tier,default_approval_mode,result_mode,requires_scope,allowed_operations_json,state FROM wrapper_capability_catalog ORDER BY capability_key",
    )?;
    let rows = statement
        .query_map([], |row| {
            let operations_json: String = row.get(6)?;
            Ok(CapabilityCatalogEntry {
                capability_key: row.get(0)?,
                description: row.get(1)?,
                risk_tier: row.get(2)?,
                default_approval_mode: row.get(3)?,
                result_mode: row.get(4)?,
                requires_scope: row.get::<_, i64>(5)? == 1,
                allowed_operations: serde_json::from_str(&operations_json).unwrap_or_default(),
                state: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_grants(connection: &Connection) -> Result<Vec<CapabilityGrant>> {
    let mut statement = connection.prepare(
        "SELECT grant_id,wrapper_id,connection_id,capability_key,grant_revision,allowed_operations_json,approval_mode,state,issued_by_user_id,reason,request_hash,not_before_utc,expires_at_utc,approved_by_user_id,approved_at_utc,revoked_at_utc,supersedes_grant_id,superseded_by_grant_id FROM wrapper_capability_grants ORDER BY updated_at_utc DESC,grant_id DESC LIMIT ?1",
    )?;
    let rows = statement
        .query_map(params![MAX_GRANTS], |row| {
            let operations_json: String = row.get(5)?;
            let revision: i64 = row.get(4)?;
            Ok(CapabilityGrant {
                grant_id: row.get(0)?,
                wrapper_id: row.get(1)?,
                connection_id: row.get(2)?,
                capability_key: row.get(3)?,
                grant_revision: revision.max(0) as u64,
                allowed_operations: serde_json::from_str(&operations_json).unwrap_or_default(),
                approval_mode: row.get(6)?,
                state: row.get(7)?,
                issued_by_user_id: row.get(8)?,
                reason: row.get(9)?,
                request_hash: row.get(10)?,
                not_before_utc: row.get(11)?,
                expires_at_utc: row.get(12)?,
                approved_by_user_id: row.get(13)?,
                approved_at_utc: row.get(14)?,
                revoked_at_utc: row.get(15)?,
                supersedes_grant_id: row.get(16)?,
                superseded_by_grant_id: row.get(17)?,
                scopes: Vec::new(),
                limits: ResourceLimits {
                    requests_per_minute: 1,
                    max_result_bytes: 1024,
                    max_daily_tokens: 0,
                    max_concurrent_jobs: 0,
                    max_queued_jobs: 0,
                    max_execution_seconds: 1,
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_scopes(connection: &Connection, grant_id: &str) -> Result<Vec<GrantScope>> {
    let mut statement = connection.prepare(
        "SELECT scope_id,grant_id,scope_kind,scope_value,allowed_fields_json,filter_json,result_policy,state FROM wrapper_dataset_scopes WHERE grant_id=?1 ORDER BY scope_kind,scope_value",
    )?;
    let rows = statement
        .query_map(params![grant_id], |row| {
            let fields_json: String = row.get(4)?;
            let filter_json: String = row.get(5)?;
            Ok(GrantScope {
                scope_id: row.get(0)?,
                grant_id: row.get(1)?,
                scope_kind: row.get(2)?,
                scope_value: row.get(3)?,
                allowed_fields: serde_json::from_str(&fields_json).unwrap_or_default(),
                filter: serde_json::from_str(&filter_json).unwrap_or_else(|_| json!({})),
                result_policy: row.get(6)?,
                state: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_scope_inputs(connection: &Connection, grant_id: &str) -> Result<Vec<ScopeInput>> {
    Ok(read_scopes(connection, grant_id)?
        .into_iter()
        .filter(|scope| scope.state == "active")
        .map(|scope| ScopeInput {
            scope_kind: scope.scope_kind,
            scope_value: scope.scope_value,
            allowed_fields: scope.allowed_fields,
            filter: scope.filter,
            result_policy: Some(scope.result_policy),
        })
        .collect())
}

fn read_limits(connection: &Connection, grant_id: &str) -> Result<ResourceLimits> {
    connection
        .query_row(
            "SELECT requests_per_minute,max_result_bytes,max_daily_tokens,max_concurrent_jobs,max_queued_jobs,max_execution_seconds FROM wrapper_resource_limits WHERE grant_id=?1",
            params![grant_id],
            limits_from_row,
        )
        .context("grant resource limits are missing")
}

fn read_limits_tx(transaction: &Transaction<'_>, grant_id: &str) -> Result<ResourceLimits> {
    transaction
        .query_row(
            "SELECT requests_per_minute,max_result_bytes,max_daily_tokens,max_concurrent_jobs,max_queued_jobs,max_execution_seconds FROM wrapper_resource_limits WHERE grant_id=?1",
            params![grant_id],
            limits_from_row,
        )
        .context("grant resource limits are missing")
}

fn limits_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ResourceLimits> {
    let result_bytes: i64 = row.get(1)?;
    let daily_tokens: i64 = row.get(2)?;
    Ok(ResourceLimits {
        requests_per_minute: row.get::<_, i64>(0)?.max(0) as u32,
        max_result_bytes: result_bytes.max(0) as u64,
        max_daily_tokens: daily_tokens.max(0) as u64,
        max_concurrent_jobs: row.get::<_, i64>(3)?.max(0) as u32,
        max_queued_jobs: row.get::<_, i64>(4)?.max(0) as u32,
        max_execution_seconds: row.get::<_, i64>(5)?.max(0) as u32,
    })
}

fn read_approvals(connection: &Connection) -> Result<Vec<GrantApproval>> {
    let mut statement = connection.prepare(
        "SELECT approval_id,grant_id,bridge_id,approval_action,plan_hash,state,requested_by_user_id,decided_by_user_id,expires_at_utc,created_at_utc,decided_at_utc,consumed_at_utc FROM wrapper_grant_approvals ORDER BY created_at_utc DESC,approval_id DESC LIMIT 2000",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(GrantApproval {
                approval_id: row.get(0)?,
                grant_id: row.get(1)?,
                bridge_id: row.get(2)?,
                approval_action: row.get(3)?,
                plan_hash: row.get(4)?,
                state: row.get(5)?,
                requested_by_user_id: row.get(6)?,
                decided_by_user_id: row.get(7)?,
                expires_at_utc: row.get(8)?,
                created_at_utc: row.get(9)?,
                decided_at_utc: row.get(10)?,
                consumed_at_utc: row.get(11)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_bridges(connection: &Connection) -> Result<Vec<BridgeGrant>> {
    let mut statement = connection.prepare(
        "SELECT bridge_id,source_wrapper_id,source_connection_id,target_wrapper_id,target_connection_id,capability_key,allowed_operations_json,scope_kind,scope_value,result_policy,approval_mode,state,issued_by_user_id,reason,request_hash,not_before_utc,expires_at_utc,approved_by_user_id,approved_at_utc,revoked_at_utc FROM wrapper_bridge_grants ORDER BY updated_at_utc DESC,bridge_id DESC LIMIT ?1",
    )?;
    let rows = statement
        .query_map(params![MAX_BRIDGES], bridge_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_broad_capabilities() {
        assert!(validate_capability_key("knowledge.search").is_ok());
        assert!(validate_capability_key("knowledge.all").is_err());
        assert!(validate_capability_key("admin").is_err());
        assert!(validate_capability_key("tools.all").is_err());
    }

    #[test]
    fn refuses_wildcard_scopes() {
        assert!(validate_scope_value("merchant:123").is_ok());
        assert!(validate_scope_value("*").is_err());
        assert!(validate_scope_value("all").is_err());
    }

    #[test]
    fn approval_mode_cannot_be_weakened() {
        let rule = CapabilityRule {
            risk_tier: "high".to_owned(),
            default_approval_mode: "explicit".to_owned(),
            result_mode: "proposed_action".to_owned(),
            requires_scope: true,
            allowed_operations: vec!["propose".to_owned()],
        };
        assert!(resolve_approval_mode(Some("none"), &rule).is_err());
        assert_eq!(
            resolve_approval_mode(Some("per_request"), &rule).expect("stronger mode"),
            "per_request"
        );
    }

    #[test]
    fn critical_grants_are_short_lived() {
        let now = Utc::now();
        assert!(validated_expiration(now, 1_440, "critical").is_ok());
        assert!(validated_expiration(now, 1_441, "critical").is_err());
    }

    #[test]
    fn operation_sets_are_normalized() {
        let operations = normalize_operations(vec![
            "Read".to_owned(),
            "read".to_owned(),
            "search".to_owned(),
        ])
        .expect("valid operations");
        assert_eq!(operations, vec!["read", "search"]);
    }
}
