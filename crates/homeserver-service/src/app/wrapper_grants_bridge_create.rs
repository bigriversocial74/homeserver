fn create_bridge(connection: &Connection, request: CreateBridgeRequest) -> Result<String> {
    expire_stale_authority(connection)?;
    let source = connection_context(connection, &request.source_connection_id)?;
    let target = connection_context(connection, &request.target_connection_id)?;
    ensure!(
        source.wrapper_id == request.source_wrapper_id,
        "source connection does not belong to the source wrapper"
    );
    ensure!(
        target.wrapper_id == request.target_wrapper_id,
        "target connection does not belong to the target wrapper"
    );
    ensure!(
        source.wrapper_id != target.wrapper_id,
        "cross-wrapper bridges require two different wrappers"
    );
    let capability_key = validate_capability_key(&request.capability_key)?;
    let rule = capability_rule(connection, &capability_key)?;
    let operations = normalize_operations(request.allowed_operations)?;
    ensure_operation_subset(&operations, &rule.allowed_operations)?;
    let scope_kind = validate_scope_kind(&request.scope_kind)?;
    let scope_value = validate_scope_value(&request.scope_value)?;
    let result_policy = validate_result_policy(&request.result_policy)?;
    let approval_mode = request
        .approval_mode
        .as_deref()
        .unwrap_or("explicit")
        .trim()
        .to_ascii_lowercase();
    ensure!(
        approval_mode == "explicit",
        "bridges require explicit approval"
    );
    ensure!(
        (5..=43_200).contains(&request.expires_minutes),
        "bridge expiration must be between 5 minutes and 30 days"
    );
    let issued_by = bounded_text(&request.issued_by_user_id, 1, 160, "issuing user")?;
    let reason = bounded_text(&request.reason, 1, 500, "bridge reason")?;
    let now = Utc::now();
    let expires_at = now + Duration::minutes(i64::from(request.expires_minutes));
    let bridge_id = Uuid::new_v4().to_string();
    let request_hash = hash_json(&json!({
        "source_wrapper_id": &source.wrapper_id,
        "source_connection_id": &source.connection_id,
        "target_wrapper_id": &target.wrapper_id,
        "target_connection_id": &target.connection_id,
        "capability_key": &capability_key,
        "operations": &operations,
        "scope_kind": &scope_kind,
        "scope_value": &scope_value,
        "result_policy": &result_policy,
        "approval_mode": &approval_mode,
        "expires_at_utc": timestamp(expires_at)
    }))?;
    let now_text = timestamp(now);
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO wrapper_bridge_grants (bridge_id,source_wrapper_id,source_connection_id,target_wrapper_id,target_connection_id,capability_key,allowed_operations_json,scope_kind,scope_value,result_policy,approval_mode,state,issued_by_user_id,reason,request_hash,not_before_utc,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'pending_approval',?12,?13,?14,?15,?16,?15,?15)",
        params![
            bridge_id,
            source.wrapper_id,
            source.connection_id,
            target.wrapper_id,
            target.connection_id,
            capability_key,
            serde_json::to_string(&operations)?,
            scope_kind,
            scope_value,
            result_policy,
            approval_mode,
            issued_by,
            reason,
            request_hash,
            now_text,
            timestamp(expires_at)
        ],
    )?;
    insert_approval(
        &transaction,
        None,
        Some(&bridge_id),
        "bridge_create",
        &request_hash,
        &issued_by,
        &timestamp(now + Duration::minutes(30)),
        &now_text,
        Some("Cross-wrapper bridge requires explicit owner approval"),
    )?;
    record_event(
        &transaction,
        &source.wrapper_id,
        &source.connection_id,
        None,
        Some(&bridge_id),
        "wrapper.bridge.created",
        "success",
        Some(&issued_by),
        "approval_required",
        json!({
            "target_wrapper_id": target.wrapper_id,
            "capability_key": capability_key,
            "expires_at_utc": timestamp(expires_at)
        }),
        &now_text,
    )?;
    transaction.commit()?;
    Ok(bridge_id)
}

