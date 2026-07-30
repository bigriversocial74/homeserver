fn expire_stale_authority(connection: &Connection) -> Result<()> {
    let now = now_utc();
    let expired_grants = {
        let mut statement = connection.prepare(
            "SELECT grant_id,wrapper_id,connection_id,capability_key FROM wrapper_capability_grants WHERE state IN ('pending_approval','active','suspended') AND expires_at_utc<=?1",
        )?;
        let rows = statement
            .query_map(params![now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let expired_bridges = {
        let mut statement = connection.prepare(
            "SELECT bridge_id,source_wrapper_id,source_connection_id,target_connection_id,capability_key FROM wrapper_bridge_grants WHERE state IN ('pending_approval','active','suspended') AND expires_at_utc<=?1",
        )?;
        let rows = statement
            .query_map(params![now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE wrapper_grant_approvals SET state='expired',decided_at_utc=?1 WHERE state IN ('pending','approved') AND expires_at_utc<=?1",
        params![now],
    )?;
    for (grant_id, wrapper_id, connection_id, capability_key) in expired_grants {
        transaction.execute(
            "UPDATE wrapper_capability_grants SET state='expired',updated_at_utc=?1 WHERE grant_id=?2",
            params![now, grant_id],
        )?;
        transaction.execute(
            "UPDATE wrapper_dataset_scopes SET state='revoked',revoked_at_utc=?1 WHERE grant_id=?2 AND state='active'",
            params![now, grant_id],
        )?;
        advance_authority_revision(&transaction, &connection_id, "grant expired", &now)?;
        record_event(
            &transaction,
            &wrapper_id,
            &connection_id,
            Some(&grant_id),
            None,
            "wrapper.grant.expired",
            "warning",
            None,
            "expired",
            json!({"capability_key": capability_key}),
            &now,
        )?;
    }
    for (bridge_id, wrapper_id, source_connection_id, target_connection_id, capability_key) in
        expired_bridges
    {
        transaction.execute(
            "UPDATE wrapper_bridge_grants SET state='expired',updated_at_utc=?1 WHERE bridge_id=?2",
            params![now, bridge_id],
        )?;
        advance_authority_revision(
            &transaction,
            &source_connection_id,
            "bridge expired",
            &now,
        )?;
        advance_authority_revision(
            &transaction,
            &target_connection_id,
            "bridge expired",
            &now,
        )?;
        record_event(
            &transaction,
            &wrapper_id,
            &source_connection_id,
            None,
            Some(&bridge_id),
            "wrapper.bridge.expired",
            "warning",
            None,
            "expired",
            json!({"capability_key": capability_key}),
            &now,
        )?;
    }
    transaction.commit()?;
    Ok(())
}

