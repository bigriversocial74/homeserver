fn revoke_bridge(connection: &Connection, request: RevokeBridgeRequest) -> Result<()> {
    ensure!(
        request.confirmation.trim() == "REVOKE BRIDGE",
        "bridge revocation requires the exact confirmation 'REVOKE BRIDGE'"
    );
    let actor = bounded_text(&request.actor_user_id, 1, 160, "revoking user")?;
    let reason = bounded_text(&request.reason, 1, 500, "revocation reason")?;
    let bridge = stored_bridge(connection, &request.bridge_id)?;
    ensure!(
        !matches!(bridge.state.as_str(), "revoked" | "expired"),
        "bridge is already inactive"
    );
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE wrapper_bridge_grants SET state='revoked',revoked_by_user_id=?1,revoked_at_utc=?2,revocation_reason=?3,updated_at_utc=?2 WHERE bridge_id=?4",
        params![actor, now, reason, bridge.bridge_id],
    )?;
    transaction.execute(
        "UPDATE wrapper_grant_approvals SET state='revoked',decided_by_user_id=?1,decided_at_utc=?2,reason=?3 WHERE bridge_id=?4 AND state IN ('pending','approved')",
        params![actor, now, reason, bridge.bridge_id],
    )?;
    advance_authority_revision(
        &transaction,
        &bridge.source_connection_id,
        "bridge revoked",
        &now,
    )?;
    advance_authority_revision(
        &transaction,
        &bridge.target_connection_id,
        "bridge revoked",
        &now,
    )?;
    record_event(
        &transaction,
        &bridge.source_wrapper_id,
        &bridge.source_connection_id,
        None,
        Some(&bridge.bridge_id),
        "wrapper.bridge.revoked",
        "success",
        Some(&actor),
        "revoked",
        json!({"target_wrapper_id": bridge.target_wrapper_id, "reason": reason}),
        &now,
    )?;
    transaction.commit()?;
    Ok(())
}

