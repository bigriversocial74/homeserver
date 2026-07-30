pub fn complete_job(
    connection: &Connection,
    request: CompleteJobRequest,
) -> Result<ExecutionReceiptSummary> {
    reconcile_authority(connection)?;
    let worker_id = validate_uuid(&request.worker_id, "worker ID")?;
    let job_id = validate_uuid(&request.job_id, "job ID")?;
    let lease_token = bounded_text(&request.lease_token, 32, 128, "lease token")?;
    let result_code = validate_symbol(&request.result_code, 120, "result code")?;
    let private_result_text = json_text(&request.private_result)?;
    ensure!(
        (2..=MAX_PRIVATE_RESULT_BYTES).contains(&private_result_text.len()),
        "private job result exceeds the HomeServer limit"
    );
    let private_provenance_text = json_text(&request.private_provenance)?;
    ensure!(
        private_provenance_text.len() <= MAX_PRIVATE_RESULT_BYTES,
        "private provenance exceeds the HomeServer limit"
    );
    let transaction = connection.unchecked_transaction()?;
    let job = validate_worker_lease_tx(
        &transaction,
        &worker_id,
        &job_id,
        &lease_token,
        &["running"],
    )?;
    ensure!(authority_is_current_tx(&transaction, &job)?, "job authority changed");
    let safe_result = project_safe_result(&job, &request.private_result)?;
    let safe_result_text = json_text(&safe_result)?;
    ensure!(
        safe_result_text.len() <= job.max_result_bytes as usize,
        "safe job result exceeds the captured grant limit"
    );
    let provenance_summary = safe_provenance_summary(
        request.source_count,
        &request.source_types,
        request.evidence_hash.as_deref(),
    )?;
    let provenance_summary_text = json_text(&provenance_summary)?;
    let private_result_hash = hash_text(&private_result_text);
    let safe_result_hash = hash_text(&safe_result_text);
    let provenance_summary_hash = hash_text(&provenance_summary_text);
    enforce_completion_usage_tx(
        &transaction,
        &job.grant_id,
        safe_result_text.len() as u64,
        request.actual_token_count.unwrap_or(0),
    )?;
    let now = now_utc();
    transaction.execute(
        "INSERT INTO wrapper_job_private_results (job_id,private_result_json,private_provenance_json,private_result_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5)",
        params![job_id, private_result_text, private_provenance_text, private_result_hash, now],
    )?;
    transaction.execute(
        "INSERT INTO wrapper_job_safe_results (job_id,result_policy,safe_result_json,safe_result_hash,provenance_summary_json,provenance_summary_hash,filter_version,result_bytes,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            job_id,
            job.result_policy,
            safe_result_text,
            safe_result_hash,
            provenance_summary_text,
            provenance_summary_hash,
            FILTER_VERSION,
            safe_result_text.len() as i64,
            now
        ],
    )?;
    transaction.execute(
        "UPDATE wrapper_jobs SET state='completed',completed_at_utc=?1,lease_owner_id=NULL,lease_token_hash=NULL,lease_expires_at_utc=NULL,failure_code=NULL,updated_at_utc=?1 WHERE job_id=?2 AND state='running'",
        params![now, job_id],
    )?;
    let completed = job_record_by_id_tx(&transaction, &job_id)?;
    record_job_event(
        &transaction,
        &completed,
        JobEventEvidence {
            event_type: "wrapper.job.completed",
            previous_state: Some("running"),
            current_state: "completed",
            outcome: "success",
            detail_code: &result_code,
            actor_type: "worker",
            actor_id: &worker_id,
            visibility: "wrapper",
            metadata: json!({
                "safe_result_hash": safe_result_hash,
                "provenance_summary_hash": provenance_summary_hash,
                "filter_version": FILTER_VERSION,
                "private_result_exposed": false
            }),
        },
    )?;
    create_terminal_receipt_tx(
        &transaction,
        &completed,
        "completed",
        &result_code,
        Some(&safe_result_hash),
        Some(&provenance_summary_hash),
        Some(&worker_id),
    )?;
    transaction.execute(
        "UPDATE wrapper_job_workers SET last_seen_at_utc=?1,updated_at_utc=?1 WHERE worker_id=?2",
        params![now, worker_id],
    )?;
    transaction.commit()?;
    read_receipt(connection, &job_id)?.context("job completion receipt was not created")
}

pub fn fail_job(connection: &Connection, request: FailJobRequest) -> Result<JobSummary> {
    reconcile_authority(connection)?;
    let worker_id = validate_uuid(&request.worker_id, "worker ID")?;
    let job_id = validate_uuid(&request.job_id, "job ID")?;
    let lease_token = bounded_text(&request.lease_token, 32, 128, "lease token")?;
    let failure_code = validate_symbol(&request.failure_code, 120, "failure code")?;
    let transaction = connection.unchecked_transaction()?;
    let job = validate_worker_lease_tx(
        &transaction,
        &worker_id,
        &job_id,
        &lease_token,
        &["leased", "running"],
    )?;
    let previous_state = job.state.clone();
    let now = Utc::now();
    let retryable = request.retryable
        && job.attempt_count < job.max_attempts
        && parse_utc(&job.expires_at_utc, "job expiration")? > now + Duration::seconds(30)
        && authority_is_current_tx(&transaction, &job)?;
    if retryable {
        let backoff_seconds = (15_i64 * 2_i64.pow(u32::from(job.attempt_count.saturating_sub(1))))
            .min(300);
        let available_at = (now + Duration::seconds(backoff_seconds))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        transaction.execute(
            "UPDATE wrapper_jobs SET state='queued',available_at_utc=?1,lease_owner_id=NULL,lease_token_hash=NULL,lease_expires_at_utc=NULL,failure_code=?2,updated_at_utc=?3 WHERE job_id=?4",
            params![available_at, failure_code, now_utc(), job_id],
        )?;
        let queued = job_record_by_id_tx(&transaction, &job_id)?;
        record_job_event(
            &transaction,
            &queued,
            JobEventEvidence {
                event_type: "wrapper.job.retry_scheduled",
                previous_state: Some(&previous_state),
                current_state: "queued",
                outcome: "warning",
                detail_code: &failure_code,
                actor_type: "worker",
                actor_id: &worker_id,
                visibility: "wrapper",
                metadata: json!({
                    "attempt_count": queued.attempt_count,
                    "max_attempts": queued.max_attempts,
                    "available_at_utc": available_at
                }),
            },
        )?;
    } else {
        let terminal_state = if request.retryable { "dead_letter" } else { "failed" };
        let completed_at = now.to_rfc3339_opts(SecondsFormat::Millis, true);
        transaction.execute(
            "UPDATE wrapper_jobs SET state=?1,completed_at_utc=?2,lease_owner_id=NULL,lease_token_hash=NULL,lease_expires_at_utc=NULL,failure_code=?3,updated_at_utc=?2 WHERE job_id=?4",
            params![terminal_state, completed_at, failure_code, job_id],
        )?;
        let terminal = job_record_by_id_tx(&transaction, &job_id)?;
        record_job_event(
            &transaction,
            &terminal,
            JobEventEvidence {
                event_type: "wrapper.job.failed",
                previous_state: Some(&previous_state),
                current_state: terminal_state,
                outcome: "error",
                detail_code: &failure_code,
                actor_type: "worker",
                actor_id: &worker_id,
                visibility: "wrapper",
                metadata: json!({
                    "attempt_count": terminal.attempt_count,
                    "max_attempts": terminal.max_attempts,
                    "private_error_exposed": false
                }),
            },
        )?;
        create_terminal_receipt_tx(
            &transaction,
            &terminal,
            terminal_state,
            &failure_code,
            None,
            None,
            Some(&worker_id),
        )?;
    }
    transaction.execute(
        "UPDATE wrapper_job_workers SET last_seen_at_utc=?1,updated_at_utc=?1 WHERE worker_id=?2",
        params![now_utc(), worker_id],
    )?;
    transaction.commit()?;
    job_summary(connection, job_record_by_id(connection, &job_id)?)
}

pub fn cancel_job(connection: &Connection, request: CancelJobRequest) -> Result<String> {
    reconcile_authority(connection)?;
    let connection_id = validate_uuid(&request.connection_id, "connection ID")?;
    let job_id = validate_uuid(&request.job_id, "job ID")?;
    let actor_type = validate_actor_type(&request.actor_type)?;
    let actor_id = bounded_text(&request.actor_id, 1, 160, "actor ID")?;
    let reason = bounded_text(&request.reason, 1, 500, "cancellation reason")?;
    let expected = format!("CANCEL JOB {job_id}");
    ensure!(request.confirmation == expected, "job cancellation confirmation is invalid");
    let transaction = connection.unchecked_transaction()?;
    let job = job_record_by_id_tx(&transaction, &job_id)?;
    ensure!(job.connection_id == connection_id, "job belongs to a different wrapper connection");
    ensure!(
        matches!(job.state.as_str(), "queued" | "leased" | "running" | "waiting"),
        "job is not cancellable"
    );
    let previous_state = job.state.clone();
    let now = now_utc();
    transaction.execute(
        "UPDATE wrapper_jobs SET state='cancelled',cancelled_at_utc=?1,completed_at_utc=?1,lease_owner_id=NULL,lease_token_hash=NULL,lease_expires_at_utc=NULL,failure_code='cancelled_by_authority',updated_at_utc=?1 WHERE job_id=?2",
        params![now, job_id],
    )?;
    let cancelled = job_record_by_id_tx(&transaction, &job_id)?;
    record_job_event(
        &transaction,
        &cancelled,
        JobEventEvidence {
            event_type: "wrapper.job.cancelled",
            previous_state: Some(&previous_state),
            current_state: "cancelled",
            outcome: "warning",
            detail_code: "cancelled_by_authority",
            actor_type: &actor_type,
            actor_id: &actor_id,
            visibility: "wrapper",
            metadata: json!({"reason": reason}),
        },
    )?;
    create_terminal_receipt_tx(
        &transaction,
        &cancelled,
        "cancelled",
        "cancelled_by_authority",
        None,
        None,
        cancelled.lease_owner_id.as_deref(),
    )?;
    transaction.commit()?;
    Ok(connection_id)
}

fn enforce_completion_usage_tx(
    transaction: &Transaction<'_>,
    grant_id: &str,
    result_bytes: u64,
    token_count: u64,
) -> Result<()> {
    let (max_result_bytes, max_daily_tokens): (i64, i64) = transaction.query_row(
        "SELECT max_result_bytes,max_daily_tokens FROM wrapper_resource_limits WHERE grant_id=?1",
        params![grant_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    ensure!(result_bytes <= max_result_bytes.max(0) as u64, "actual safe result exceeds the grant limit");
    let day_start = Utc::now().format("%Y-%m-%dT00:00:00.000Z").to_string();
    let current_tokens: i64 = transaction
        .query_row(
            "SELECT token_count FROM wrapper_grant_usage_windows WHERE grant_id=?1 AND window_kind='day' AND window_start_utc=?2",
            params![grant_id, day_start],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    ensure!(
        current_tokens.saturating_add(token_count as i64) <= max_daily_tokens,
        "actual token use exceeds the daily grant limit"
    );
    transaction.execute(
        "INSERT INTO wrapper_grant_usage_windows (grant_id,window_kind,window_start_utc,request_count,result_bytes,token_count,updated_at_utc) VALUES (?1,'day',?2,0,?3,?4,?5) ON CONFLICT(grant_id,window_kind,window_start_utc) DO UPDATE SET result_bytes=result_bytes+excluded.result_bytes,token_count=token_count+excluded.token_count,updated_at_utc=excluded.updated_at_utc",
        params![grant_id, day_start, result_bytes as i64, token_count as i64, now_utc()],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn create_terminal_receipt_tx(
    transaction: &Transaction<'_>,
    job: &JobRecord,
    outcome: &str,
    result_code: &str,
    safe_result_hash: Option<&str>,
    provenance_summary_hash: Option<&str>,
    worker_id: Option<&str>,
) -> Result<String> {
    ensure!(TERMINAL_STATES.contains(&outcome), "receipt outcome is not terminal");
    let receipt_id = Uuid::new_v4().to_string();
    let completed_at = job.completed_at_utc.clone().unwrap_or_else(now_utc);
    let worker_kind: Option<String> = worker_id
        .map(|worker_id| {
            transaction
                .query_row(
                    "SELECT worker_kind FROM wrapper_job_workers WHERE worker_id=?1",
                    params![worker_id],
                    |row| row.get(0),
                )
                .optional()
                .map(|value| value.flatten())
        })
        .transpose()?
        .flatten();
    let receipt_document = json!({
        "schema": "homeserver.wrapper-job-receipt.v1",
        "receipt_id": receipt_id,
        "job_id": job.job_id,
        "wrapper_id": job.wrapper_id,
        "connection_id": job.connection_id,
        "grant_id": job.grant_id,
        "grant_revision": job.grant_revision,
        "connection_authority_revision": job.connection_authority_revision,
        "authorization_decision_id": job.authorization_decision_id,
        "capability_key": job.capability_key,
        "operation": job.operation,
        "job_type": job.job_type,
        "idempotency_key": job.idempotency_key,
        "request_hash": job.request_hash,
        "payload_hash": job.payload_hash,
        "approval_id": job.approval_id,
        "plan_hash": job.plan_hash,
        "correlation_id": job.correlation_id,
        "causation_id": job.causation_id,
        "outcome": outcome,
        "result_code": result_code,
        "safe_result_hash": safe_result_hash,
        "provenance_summary_hash": provenance_summary_hash,
        "worker_id": worker_id,
        "worker_kind": worker_kind,
        "attempt_count": job.attempt_count,
        "started_at_utc": job.started_at_utc,
        "completed_at_utc": completed_at
    });
    let receipt_hash = hash_json(&receipt_document)?;
    transaction.execute(
        "INSERT INTO wrapper_job_execution_receipts (receipt_id,job_id,wrapper_id,connection_id,grant_id,grant_revision,authorization_decision_id,capability_key,operation,job_type,idempotency_key,request_hash,payload_hash,approval_id,plan_hash,correlation_id,causation_id,outcome,result_code,safe_result_hash,provenance_summary_hash,worker_id,worker_kind,attempt_count,started_at_utc,completed_at_utc,receipt_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?26)",
        params![
            receipt_id,
            job.job_id,
            job.wrapper_id,
            job.connection_id,
            job.grant_id,
            job.grant_revision as i64,
            job.authorization_decision_id,
            job.capability_key,
            job.operation,
            job.job_type,
            job.idempotency_key,
            job.request_hash,
            job.payload_hash,
            job.approval_id,
            job.plan_hash,
            job.correlation_id,
            job.causation_id,
            outcome,
            result_code,
            safe_result_hash,
            provenance_summary_hash,
            worker_id,
            worker_kind,
            i64::from(job.attempt_count),
            job.started_at_utc,
            completed_at,
            receipt_hash
        ],
    )?;
    let delivery_id = Uuid::new_v4().to_string();
    let delivery_expires = (Utc::now() + Duration::days(30))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    transaction.execute(
        "INSERT INTO wrapper_job_deliveries (delivery_id,job_id,receipt_id,connection_id,state,payload_hash,attempt_count,next_attempt_at_utc,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,'pending',?5,0,?6,?7,?6,?6)",
        params![delivery_id, job.job_id, receipt_id, job.connection_id, receipt_hash, completed_at, delivery_expires],
    )?;
    Ok(receipt_id)
}
