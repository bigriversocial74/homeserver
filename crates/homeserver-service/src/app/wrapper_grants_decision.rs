struct DeniedAuthorization<'a> {
    grant_id: Option<&'a str>,
    bridge_id: Option<&'a str>,
    capability_key: &'a str,
    operation: &'a str,
    detail_code: &'a str,
    correlation_id: Option<&'a str>,
    grant_revision: Option<u64>,
}

fn denied_decision(
    connection: &Connection,
    context: &ConnectionContext,
    denied: DeniedAuthorization<'_>,
) -> Result<AuthorizationDecision> {
    let decision_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO wrapper_authorization_receipts (decision_id,wrapper_id,connection_id,grant_id,bridge_id,capability_key,operation,outcome,detail_code,grant_revision,correlation_id,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,'denied',?8,?9,?10,?11)",
        params![
            decision_id,
            context.wrapper_id,
            context.connection_id,
            denied.grant_id,
            denied.bridge_id,
            denied.capability_key,
            denied.operation,
            denied.detail_code,
            denied.grant_revision.unwrap_or(context.grant_revision) as i64,
            denied.correlation_id,
            now_utc()
        ],
    )?;
    Ok(AuthorizationDecision {
        allowed: false,
        decision_id,
        wrapper_id: context.wrapper_id.clone(),
        connection_id: context.connection_id.clone(),
        grant_id: denied.grant_id.map(ToOwned::to_owned),
        bridge_id: denied.bridge_id.map(ToOwned::to_owned),
        capability_key: denied.capability_key.to_owned(),
        operation: denied.operation.to_owned(),
        grant_revision: denied.grant_revision.unwrap_or(context.grant_revision),
        result_policy: None,
        expires_at_utc: None,
        detail_code: denied.detail_code.to_owned(),
    })
}
