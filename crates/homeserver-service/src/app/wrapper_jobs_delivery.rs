pub fn poll_deliveries(
    connection: &Connection,
    request: PollDeliveriesRequest,
) -> Result<Vec<DeliveryEnvelope>> {
    reconcile_authority(connection)?;
    let connection_id = validate_uuid(&request.connection_id, "connection ID")?;
    let exists: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_connections c JOIN wrapper_identities w ON w.wrapper_id=c.wrapper_id WHERE c.connection_id=?1 AND c.lifecycle_state IN ('active','offline','grace') AND w.state='active'",
        params![connection_id],
        |row| row.get(0),
    )?;
    ensure!(exists == 1, "active wrapper connection was not found");
    let limit = i64::from(request.limit.unwrap_or(25)).clamp(1, MAX_DELIVERIES_PER_POLL);
    let now = now_utc();
    let mut statement = connection.prepare(
        "SELECT delivery_id,job_id,receipt_id,connection_id,state,payload_hash,attempt_count,next_attempt_at_utc,acknowledged_at_utc,expires_at_utc FROM wrapper_job_deliveries WHERE connection_id=?1 AND state IN ('pending','in_flight') AND next_attempt_at_utc<=?2 AND expires_at_utc>?2 ORDER BY created_at_utc,delivery_id LIMIT ?3",
    )?;
    let deliveries = statement
        .query_map(params![connection_id, now, limit], delivery_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let transaction = connection.unchecked_transaction()?;
    for delivery in &deliveries {
        let delay_seconds = (15_i64 * 2_i64.pow(delivery.attempt_count.min(8))).min(3_600);
        let next_attempt = (Utc::now() + Duration::seconds(delay_seconds))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        transaction.execute(
            "UPDATE wrapper_job_deliveries SET state='in_flight',attempt_count=attempt_count+1,last_attempt_at_utc=?1,next_attempt_at_utc=?2,updated_at_utc=?1 WHERE delivery_id=?3 AND connection_id=?4 AND state IN ('pending','in_flight')",
            params![now, next_attempt, delivery.delivery_id, connection_id],
        )?;
    }
    transaction.commit()?;
    deliveries
        .into_iter()
        .map(|delivery| {
            let updated = read_delivery(connection, &delivery.delivery_id)?;
            let job = job_summary(connection, job_record_by_id(connection, &delivery.job_id)?)?;
            Ok(DeliveryEnvelope {
                delivery: updated,
                job,
            })
        })
        .collect()
}

pub fn acknowledge_delivery(
    connection: &Connection,
    request: AckDeliveryRequest,
) -> Result<DeliverySummary> {
    let connection_id = validate_uuid(&request.connection_id, "connection ID")?;
    let delivery_id = validate_uuid(&request.delivery_id, "delivery ID")?;
    let receipt_hash = validate_sha256(&request.receipt_hash, "receipt hash")?;
    let transaction = connection.unchecked_transaction()?;
    let (job_id, stored_hash, state): (String, String, String) = transaction
        .query_row(
            "SELECT d.job_id,r.receipt_hash,d.state FROM wrapper_job_deliveries d JOIN wrapper_job_execution_receipts r ON r.receipt_id=d.receipt_id WHERE d.delivery_id=?1 AND d.connection_id=?2",
            params![delivery_id, connection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .context("wrapper delivery was not found")?;
    ensure!(
        matches!(state.as_str(), "pending" | "in_flight"),
        "wrapper delivery is not awaiting acknowledgement"
    );
    ensure!(stored_hash == receipt_hash, "delivery receipt hash does not match");
    let now = now_utc();
    transaction.execute(
        "UPDATE wrapper_job_deliveries SET state='acknowledged',acknowledged_at_utc=?1,updated_at_utc=?1 WHERE delivery_id=?2 AND connection_id=?3",
        params![now, delivery_id, connection_id],
    )?;
    let job = job_record_by_id_tx(&transaction, &job_id)?;
    record_job_event(
        &transaction,
        &job,
        JobEventEvidence {
            event_type: "wrapper.job.delivery_acknowledged",
            previous_state: Some(&job.state),
            current_state: &job.state,
            outcome: "success",
            detail_code: "receipt_hash_confirmed",
            actor_type: "wrapper",
            actor_id: &connection_id,
            visibility: "internal",
            metadata: json!({"delivery_id": delivery_id, "receipt_hash": receipt_hash}),
        },
    )?;
    transaction.commit()?;
    read_delivery(connection, &delivery_id)
}

fn read_delivery(connection: &Connection, delivery_id: &str) -> Result<DeliverySummary> {
    connection
        .query_row(
            "SELECT delivery_id,job_id,receipt_id,connection_id,state,payload_hash,attempt_count,next_attempt_at_utc,acknowledged_at_utc,expires_at_utc FROM wrapper_job_deliveries WHERE delivery_id=?1",
            params![delivery_id],
            delivery_from_row,
        )
        .context("wrapper delivery was not found")
}
