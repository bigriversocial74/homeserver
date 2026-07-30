pub fn authorize_bridge(
    connection: &Connection,
    request: AuthorizeBridgeRequest,
) -> Result<AuthorizationDecision> {
    expire_stale_authority(connection)?;
    let source = connection_context(connection, &request.source_connection_id)?;
    let target = connection_context(connection, &request.target_connection_id)?;
    let capability_key = validate_capability_key(&request.capability_key)?;
    let operation = validate_operation(&request.operation)?;
    let scope_kind = validate_scope_kind(&request.scope_kind)?;
    let scope_value = validate_scope_value(&request.scope_value)?;
    let correlation_id = request
        .correlation_id
        .as_deref()
        .map(|value| bounded_text(value, 1, 160, "correlation ID"))
        .transpose()?;
    let bridge: Option<BridgeGrant> = connection
        .query_row(
            "SELECT bridge_id,source_wrapper_id,source_connection_id,target_wrapper_id,target_connection_id,capability_key,allowed_operations_json,scope_kind,scope_value,result_policy,approval_mode,state,issued_by_user_id,reason,request_hash,not_before_utc,expires_at_utc,approved_by_user_id,approved_at_utc,revoked_at_utc FROM wrapper_bridge_grants WHERE source_connection_id=?1 AND target_connection_id=?2 AND capability_key=?3 AND scope_kind=?4 AND scope_value=?5 AND state='active' AND not_before_utc<=?6 AND expires_at_utc>?6 ORDER BY created_at_utc DESC LIMIT 1",
            params![source.connection_id,target.connection_id,capability_key,scope_kind,scope_value,now_utc()],
            bridge_from_row,
        )
        .optional()?;
    let Some(bridge) = bridge else {
        return denied_decision(
            connection,
            &source,
            None,
            None,
            &capability_key,
            &operation,
            "bridge_missing",
            correlation_id.as_deref(),
            None,
        );
    };
    if !bridge.allowed_operations.iter().any(|item| item == &operation) {
        return denied_decision(
            connection,
            &source,
            None,
            Some(&bridge.bridge_id),
            &capability_key,
            &operation,
            "bridge_operation_not_granted",
            correlation_id.as_deref(),
            None,
        );
    }
    let transaction = connection.unchecked_transaction()?;
    let decision_id = Uuid::new_v4().to_string();
    transaction.execute(
        "INSERT INTO wrapper_authorization_receipts (decision_id,wrapper_id,connection_id,bridge_id,capability_key,operation,outcome,detail_code,grant_revision,scope_hash,result_policy,correlation_id,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,'allowed','bridge_authorized',?7,?8,?9,?10,?11)",
        params![
            decision_id,
            source.wrapper_id,
            source.connection_id,
            bridge.bridge_id,
            capability_key,
            operation,
            source.grant_revision as i64,
            hash_text(&format!("{}:{}", scope_kind, scope_value)),
            bridge.result_policy,
            correlation_id,
            now_utc()
        ],
    )?;
    transaction.commit()?;
    Ok(AuthorizationDecision {
        allowed: true,
        decision_id,
        wrapper_id: source.wrapper_id,
        connection_id: source.connection_id,
        grant_id: None,
        bridge_id: Some(bridge.bridge_id),
        capability_key,
        operation,
        grant_revision: source.grant_revision,
        result_policy: Some(bridge.result_policy),
        expires_at_utc: Some(bridge.expires_at_utc),
        detail_code: "bridge_authorized".to_owned(),
    })
}

