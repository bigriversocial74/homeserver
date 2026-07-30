fn denied_decision(
    connection: &Connection,
    context: &ConnectionContext,
    grant_id: Option<&str>,
    bridge_id: Option<&str>,
    capability_key: &str,
    operation: &str,
    detail_code: &str,
    correlation_id: Option<&str>,
    grant_revision: Option<u64>,
) -> Result<AuthorizationDecision> {
    let decision_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO wrapper_authorization_receipts (decision_id,wrapper_id,connection_id,grant_id,bridge_id,capability_key,operation,outcome,detail_code,grant_revision,correlation_id,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,'denied',?8,?9,?10,?11)",
        params![
            decision_id,
            context.wrapper_id,
            context.connection_id,
            grant_id,
            bridge_id,
            capability_key,
            operation,
            detail_code,
            grant_revision.unwrap_or(context.grant_revision) as i64,
            correlation_id,
            now_utc()
        ],
    )?;
    Ok(AuthorizationDecision {
        allowed: false,
        decision_id,
        wrapper_id: context.wrapper_id.clone(),
        connection_id: context.connection_id.clone(),
        grant_id: grant_id.map(ToOwned::to_owned),
        bridge_id: bridge_id.map(ToOwned::to_owned),
        capability_key: capability_key.to_owned(),
        operation: operation.to_owned(),
        grant_revision: grant_revision.unwrap_or(context.grant_revision),
        result_policy: None,
        expires_at_utc: None,
        detail_code: detail_code.to_owned(),
    })
}

