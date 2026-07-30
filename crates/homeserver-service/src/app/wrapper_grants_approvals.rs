fn decide_approval(connection: &Connection, request: DecideApprovalRequest) -> Result<()> {
    let actor = bounded_text(&request.actor_user_id, 1, 160, "approving user")?;
    let plan_hash = validate_sha256(&request.plan_hash, "plan hash")?;
    let decision = request.decision.trim().to_ascii_lowercase();
    ensure!(
        decision == "approve" || decision == "reject",
        "approval decision must be approve or reject"
    );
    let expected_confirmation = if decision == "approve" {
        "APPROVE"
    } else {
        "REJECT"
    };
    ensure!(
        request.confirmation.trim() == expected_confirmation,
        "approval decision confirmation does not match the requested decision"
    );
    let now = now_utc();
    let approval: (Option<String>, Option<String>, String, String, String, String) = connection
        .query_row(
            "SELECT grant_id,bridge_id,approval_action,plan_hash,state,expires_at_utc FROM wrapper_grant_approvals WHERE approval_id=?1",
            params![request.approval_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
        )
        .context("approval was not found")?;
    ensure!(approval.4 == "pending", "approval is not pending");
    ensure!(approval.3 == plan_hash, "approval plan hash does not match");
    ensure!(approval.5 > now, "approval has expired");
    let transaction = connection.unchecked_transaction()?;
    if decision == "reject" {
        transaction.execute(
            "UPDATE wrapper_grant_approvals SET state='rejected',decided_by_user_id=?1,decided_at_utc=?2,reason=?3 WHERE approval_id=?4",
            params![actor, now, request.reason, request.approval_id],
        )?;
        if let Some(grant_id) = approval.0 {
            transaction.execute(
                "UPDATE wrapper_capability_grants SET state='revoked',revoked_by_user_id=?1,revoked_at_utc=?2,revocation_reason='approval rejected',updated_at_utc=?2 WHERE grant_id=?3 AND state='pending_approval'",
                params![actor, now, grant_id],
            )?;
        }
        if let Some(bridge_id) = approval.1 {
            transaction.execute(
                "UPDATE wrapper_bridge_grants SET state='revoked',revoked_by_user_id=?1,revoked_at_utc=?2,revocation_reason='approval rejected',updated_at_utc=?2 WHERE bridge_id=?3 AND state='pending_approval'",
                params![actor, now, bridge_id],
            )?;
        }
        transaction.commit()?;
        return Ok(());
    }

    transaction.execute(
        "UPDATE wrapper_grant_approvals SET state='approved',decided_by_user_id=?1,decided_at_utc=?2,reason=?3 WHERE approval_id=?4",
        params![actor, now, request.reason, request.approval_id],
    )?;
    match approval.2.as_str() {
        "grant_create" | "grant_rotate" => {
            let grant_id = approval.0.context("grant approval target is missing")?;
            let grant = stored_grant_tx(&transaction, &grant_id)?;
            ensure!(grant.state == "pending_approval", "grant is not pending approval");
            activate_supersession(
                &transaction,
                &grant.grant_id,
                grant.supersedes_grant_id.as_deref(),
                &now,
            )?;
            transaction.execute(
                "UPDATE wrapper_capability_grants SET state='active',approved_by_user_id=?1,approved_at_utc=?2,updated_at_utc=?2 WHERE grant_id=?3",
                params![actor, now, grant.grant_id],
            )?;
            advance_authority_revision(
                &transaction,
                &grant.connection_id,
                "grant approved",
                &now,
            )?;
            record_event(
                &transaction,
                &grant.wrapper_id,
                &grant.connection_id,
                Some(&grant.grant_id),
                None,
                "wrapper.grant.approved",
                "success",
                Some(&actor),
                "active",
                json!({"capability_key": grant.capability_key}),
                &now,
            )?;
        }
        "bridge_create" => {
            let bridge_id = approval.1.context("bridge approval target is missing")?;
            let bridge = stored_bridge_tx(&transaction, &bridge_id)?;
            ensure!(
                bridge.state == "pending_approval",
                "bridge is not pending approval"
            );
            transaction.execute(
                "UPDATE wrapper_bridge_grants SET state='active',approved_by_user_id=?1,approved_at_utc=?2,updated_at_utc=?2 WHERE bridge_id=?3",
                params![actor, now, bridge.bridge_id],
            )?;
            advance_authority_revision(
                &transaction,
                &bridge.source_connection_id,
                "bridge approved",
                &now,
            )?;
            advance_authority_revision(
                &transaction,
                &bridge.target_connection_id,
                "bridge approved",
                &now,
            )?;
            record_event(
                &transaction,
                &bridge.source_wrapper_id,
                &bridge.source_connection_id,
                None,
                Some(&bridge.bridge_id),
                "wrapper.bridge.approved",
                "success",
                Some(&actor),
                "active",
                json!({
                    "target_wrapper_id": bridge.target_wrapper_id,
                    "capability_key": bridge.capability_key
                }),
                &now,
            )?;
        }
        "sensitive_use" => {}
        _ => bail!("unsupported approval action"),
    }
    transaction.commit()?;
    Ok(())
}

