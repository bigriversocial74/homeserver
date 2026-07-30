pub fn authorize(connection: &Connection, request: AuthorizeRequest) -> Result<AuthorizationDecision> {
    expire_stale_authority(connection)?;
    let context = connection_context(connection, &request.connection_id)?;
    let capability_key = validate_capability_key(&request.capability_key)?;
    let operation = validate_operation(&request.operation)?;
    let correlation_id = request
        .correlation_id
        .as_deref()
        .map(|value| bounded_text(value, 1, 160, "correlation ID"))
        .transpose()?;
    let grant = active_grant(connection, &context.connection_id, &capability_key)?;
    let Some(grant) = grant else {
        return denied_decision(
            connection,
            &context,
            None,
            None,
            &capability_key,
            &operation,
            "grant_missing",
            correlation_id.as_deref(),
            None,
        );
    };
    if !grant.allowed_operations.iter().any(|item| item == &operation) {
        return denied_decision(
            connection,
            &context,
            Some(&grant.grant_id),
            None,
            &capability_key,
            &operation,
            "operation_not_granted",
            correlation_id.as_deref(),
            Some(grant.grant_revision),
        );
    }
    let rule = capability_rule(connection, &capability_key)?;
    let scope_policy = if rule.requires_scope {
        let scope_kind = request
            .scope_kind
            .as_deref()
            .context("scope kind is required")?;
        let scope_value = request
            .scope_value
            .as_deref()
            .context("scope value is required")?;
        match matching_scope(connection, &grant.grant_id, scope_kind, scope_value)? {
            Some(policy) => Some(policy),
            None => {
                return denied_decision(
                    connection,
                    &context,
                    Some(&grant.grant_id),
                    None,
                    &capability_key,
                    &operation,
                    "scope_not_granted",
                    correlation_id.as_deref(),
                    Some(grant.grant_revision),
                )
            }
        }
    } else {
        None
    };
    let transaction = connection.unchecked_transaction()?;
    if grant.approval_mode == "per_request" {
        let approval_id = request
            .approval_id
            .as_deref()
            .context("per-request approval is required")?;
        let plan_hash = request
            .plan_hash
            .as_deref()
            .map(|value| validate_sha256(value, "plan hash"))
            .transpose()?
            .context("per-request plan hash is required")?;
        consume_use_approval(&transaction, &grant.grant_id, approval_id, &plan_hash)?;
    }
    enforce_and_record_usage(
        &transaction,
        &grant.grant_id,
        request.result_bytes.unwrap_or(0),
        request.token_count.unwrap_or(0),
    )?;
    let decision_id = Uuid::new_v4().to_string();
    let scope_hash = request
        .scope_kind
        .as_deref()
        .zip(request.scope_value.as_deref())
        .map(|(kind, value)| hash_text(&format!("{kind}:{value}")));
    transaction.execute(
        "INSERT INTO wrapper_authorization_receipts (decision_id,wrapper_id,connection_id,grant_id,capability_key,operation,outcome,detail_code,grant_revision,scope_hash,result_policy,correlation_id,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,'allowed','authorized',?7,?8,?9,?10,?11)",
        params![
            decision_id,
            context.wrapper_id,
            context.connection_id,
            grant.grant_id,
            capability_key,
            operation,
            grant.grant_revision as i64,
            scope_hash,
            scope_policy.as_deref().unwrap_or(&rule.result_mode),
            correlation_id,
            now_utc()
        ],
    )?;
    transaction.commit()?;
    Ok(AuthorizationDecision {
        allowed: true,
        decision_id,
        wrapper_id: context.wrapper_id,
        connection_id: context.connection_id,
        grant_id: Some(grant.grant_id),
        bridge_id: None,
        capability_key,
        operation,
        grant_revision: grant.grant_revision,
        result_policy: Some(scope_policy.unwrap_or(rule.result_mode)),
        expires_at_utc: Some(grant.expires_at_utc),
        detail_code: "authorized".to_owned(),
    })
}

