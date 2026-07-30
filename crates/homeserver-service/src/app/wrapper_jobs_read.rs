const JOB_RECORD_SELECT: &str = "SELECT j.job_id,j.wrapper_id,j.connection_id,j.grant_id,j.grant_revision,a.connection_authority_revision,j.authorization_decision_id,j.capability_key,j.operation,j.job_type,j.state,j.priority,j.idempotency_key,j.request_hash,j.payload_hash,j.scope_kind,j.scope_value,j.result_policy,j.allowed_result_fields_json,j.max_result_bytes,j.max_execution_seconds,j.max_attempts,j.attempt_count,j.approval_id,j.plan_hash,j.correlation_id,j.causation_id,j.submitted_by_type,j.submitted_by_id,j.available_at_utc,j.expires_at_utc,j.lease_owner_id,j.lease_token_hash,j.lease_expires_at_utc,j.started_at_utc,j.completed_at_utc,j.cancelled_at_utc,j.failure_code,j.created_at_utc FROM wrapper_jobs j JOIN wrapper_job_authority_snapshots a ON a.job_id=j.job_id";
const JOB_RECORD_QUERY_BY_ID: &str = "SELECT j.job_id,j.wrapper_id,j.connection_id,j.grant_id,j.grant_revision,a.connection_authority_revision,j.authorization_decision_id,j.capability_key,j.operation,j.job_type,j.state,j.priority,j.idempotency_key,j.request_hash,j.payload_hash,j.scope_kind,j.scope_value,j.result_policy,j.allowed_result_fields_json,j.max_result_bytes,j.max_execution_seconds,j.max_attempts,j.attempt_count,j.approval_id,j.plan_hash,j.correlation_id,j.causation_id,j.submitted_by_type,j.submitted_by_id,j.available_at_utc,j.expires_at_utc,j.lease_owner_id,j.lease_token_hash,j.lease_expires_at_utc,j.started_at_utc,j.completed_at_utc,j.cancelled_at_utc,j.failure_code,j.created_at_utc FROM wrapper_jobs j JOIN wrapper_job_authority_snapshots a ON a.job_id=j.job_id WHERE j.job_id=?1";
const JOB_RECORD_QUERY_BY_IDEMPOTENCY: &str = "SELECT j.job_id,j.wrapper_id,j.connection_id,j.grant_id,j.grant_revision,a.connection_authority_revision,j.authorization_decision_id,j.capability_key,j.operation,j.job_type,j.state,j.priority,j.idempotency_key,j.request_hash,j.payload_hash,j.scope_kind,j.scope_value,j.result_policy,j.allowed_result_fields_json,j.max_result_bytes,j.max_execution_seconds,j.max_attempts,j.attempt_count,j.approval_id,j.plan_hash,j.correlation_id,j.causation_id,j.submitted_by_type,j.submitted_by_id,j.available_at_utc,j.expires_at_utc,j.lease_owner_id,j.lease_token_hash,j.lease_expires_at_utc,j.started_at_utc,j.completed_at_utc,j.cancelled_at_utc,j.failure_code,j.created_at_utc FROM wrapper_jobs j JOIN wrapper_job_authority_snapshots a ON a.job_id=j.job_id WHERE j.connection_id=?1 AND j.idempotency_key=?2";

fn job_record_from_row(row: &Row<'_>) -> rusqlite::Result<JobRecord> {
    let grant_revision: i64 = row.get(4)?;
    let connection_authority_revision: i64 = row.get(5)?;
    let priority: i64 = row.get(11)?;
    let max_result_bytes: i64 = row.get(19)?;
    let max_execution_seconds: i64 = row.get(20)?;
    let max_attempts: i64 = row.get(21)?;
    let attempt_count: i64 = row.get(22)?;
    let allowed_fields_json: String = row.get(18)?;
    Ok(JobRecord {
        job_id: row.get(0)?,
        wrapper_id: row.get(1)?,
        connection_id: row.get(2)?,
        grant_id: row.get(3)?,
        grant_revision: grant_revision.max(0) as u64,
        connection_authority_revision: connection_authority_revision.max(0) as u64,
        authorization_decision_id: row.get(6)?,
        capability_key: row.get(7)?,
        operation: row.get(8)?,
        job_type: row.get(9)?,
        state: row.get(10)?,
        priority: priority.clamp(0, 9) as u8,
        idempotency_key: row.get(12)?,
        request_hash: row.get(13)?,
        payload_hash: row.get(14)?,
        scope_kind: row.get(15)?,
        scope_value: row.get(16)?,
        result_policy: row.get(17)?,
        allowed_result_fields: serde_json::from_str(&allowed_fields_json).unwrap_or_default(),
        max_result_bytes: max_result_bytes.max(0) as u64,
        max_execution_seconds: max_execution_seconds.max(0) as u32,
        max_attempts: max_attempts.clamp(0, 20) as u8,
        attempt_count: attempt_count.clamp(0, 20) as u8,
        approval_id: row.get(23)?,
        plan_hash: row.get(24)?,
        correlation_id: row.get(25)?,
        causation_id: row.get(26)?,
        submitted_by_type: row.get(27)?,
        submitted_by_id: row.get(28)?,
        available_at_utc: row.get(29)?,
        expires_at_utc: row.get(30)?,
        lease_owner_id: row.get(31)?,
        lease_token_hash: row.get(32)?,
        lease_expires_at_utc: row.get(33)?,
        started_at_utc: row.get(34)?,
        completed_at_utc: row.get(35)?,
        cancelled_at_utc: row.get(36)?,
        failure_code: row.get(37)?,
        created_at_utc: row.get(38)?,
    })
}

fn job_record_by_id(connection: &Connection, job_id: &str) -> Result<JobRecord> {
    connection
        .query_row(JOB_RECORD_QUERY_BY_ID, params![job_id], job_record_from_row)
        .context("wrapper job was not found")
}

fn job_record_by_id_tx(transaction: &Transaction<'_>, job_id: &str) -> Result<JobRecord> {
    transaction
        .query_row(JOB_RECORD_QUERY_BY_ID, params![job_id], job_record_from_row)
        .context("wrapper job was not found")
}

fn job_summary(connection: &Connection, job: JobRecord) -> Result<JobSummary> {
    let safe_result = read_safe_result(connection, &job.job_id)?;
    let receipt = read_receipt(connection, &job.job_id)?;
    Ok(JobSummary {
        job_id: job.job_id,
        wrapper_id: job.wrapper_id,
        connection_id: job.connection_id,
        grant_id: job.grant_id,
        grant_revision: job.grant_revision,
        connection_authority_revision: job.connection_authority_revision,
        authorization_decision_id: job.authorization_decision_id,
        capability_key: job.capability_key,
        operation: job.operation,
        job_type: job.job_type,
        state: job.state,
        priority: job.priority,
        idempotency_key: job.idempotency_key,
        request_hash: job.request_hash,
        payload_hash: job.payload_hash,
        scope_kind: job.scope_kind,
        scope_value: job.scope_value,
        result_policy: job.result_policy,
        allowed_result_fields: job.allowed_result_fields,
        max_result_bytes: job.max_result_bytes,
        max_execution_seconds: job.max_execution_seconds,
        max_attempts: job.max_attempts,
        attempt_count: job.attempt_count,
        approval_id: job.approval_id,
        plan_hash: job.plan_hash,
        correlation_id: job.correlation_id,
        causation_id: job.causation_id,
        submitted_by_type: job.submitted_by_type,
        submitted_by_id: job.submitted_by_id,
        available_at_utc: job.available_at_utc,
        expires_at_utc: job.expires_at_utc,
        lease_owner_id: job.lease_owner_id,
        lease_expires_at_utc: job.lease_expires_at_utc,
        started_at_utc: job.started_at_utc,
        completed_at_utc: job.completed_at_utc,
        cancelled_at_utc: job.cancelled_at_utc,
        failure_code: job.failure_code,
        created_at_utc: job.created_at_utc,
        safe_result,
        receipt,
    })
}

fn read_safe_result(connection: &Connection, job_id: &str) -> Result<Option<SafeResultSummary>> {
    connection
        .query_row(
            "SELECT job_id,result_policy,safe_result_json,safe_result_hash,provenance_summary_json,provenance_summary_hash,filter_version,result_bytes FROM wrapper_job_safe_results WHERE job_id=?1",
            params![job_id],
            |row| {
                let safe_json: String = row.get(2)?;
                let provenance_json: String = row.get(4)?;
                let result_bytes: i64 = row.get(7)?;
                Ok(SafeResultSummary {
                    job_id: row.get(0)?,
                    result_policy: row.get(1)?,
                    safe_result: serde_json::from_str(&safe_json).unwrap_or_else(|_| json!({})),
                    safe_result_hash: row.get(3)?,
                    provenance_summary: serde_json::from_str(&provenance_json)
                        .unwrap_or_else(|_| json!({})),
                    provenance_summary_hash: row.get(5)?,
                    filter_version: row.get(6)?,
                    result_bytes: result_bytes.max(0) as u64,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn read_receipt(connection: &Connection, job_id: &str) -> Result<Option<ExecutionReceiptSummary>> {
    connection
        .query_row(
            "SELECT r.receipt_id,r.job_id,r.wrapper_id,r.connection_id,r.grant_id,r.grant_revision,a.connection_authority_revision,r.authorization_decision_id,r.capability_key,r.operation,r.job_type,r.idempotency_key,r.request_hash,r.payload_hash,r.approval_id,r.plan_hash,r.correlation_id,r.causation_id,r.outcome,r.result_code,r.safe_result_hash,r.provenance_summary_hash,r.worker_id,r.worker_kind,r.attempt_count,r.started_at_utc,r.completed_at_utc,r.receipt_hash FROM wrapper_job_execution_receipts r JOIN wrapper_job_authority_snapshots a ON a.job_id=r.job_id WHERE r.job_id=?1",
            params![job_id],
            |row| {
                let grant_revision: i64 = row.get(5)?;
                let authority_revision: i64 = row.get(6)?;
                let attempt_count: i64 = row.get(24)?;
                Ok(ExecutionReceiptSummary {
                    receipt_id: row.get(0)?,
                    job_id: row.get(1)?,
                    wrapper_id: row.get(2)?,
                    connection_id: row.get(3)?,
                    grant_id: row.get(4)?,
                    grant_revision: grant_revision.max(0) as u64,
                    connection_authority_revision: authority_revision.max(0) as u64,
                    authorization_decision_id: row.get(7)?,
                    capability_key: row.get(8)?,
                    operation: row.get(9)?,
                    job_type: row.get(10)?,
                    idempotency_key: row.get(11)?,
                    request_hash: row.get(12)?,
                    payload_hash: row.get(13)?,
                    approval_id: row.get(14)?,
                    plan_hash: row.get(15)?,
                    correlation_id: row.get(16)?,
                    causation_id: row.get(17)?,
                    outcome: row.get(18)?,
                    result_code: row.get(19)?,
                    safe_result_hash: row.get(20)?,
                    provenance_summary_hash: row.get(21)?,
                    worker_id: row.get(22)?,
                    worker_kind: row.get(23)?,
                    attempt_count: attempt_count.max(0) as u32,
                    started_at_utc: row.get(25)?,
                    completed_at_utc: row.get(26)?,
                    receipt_hash: row.get(27)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn read_event(row: &Row<'_>) -> rusqlite::Result<JobEventSummary> {
    let sequence: i64 = row.get(2)?;
    let metadata_json: String = row.get(10)?;
    Ok(JobEventSummary {
        event_id: row.get(0)?,
        job_id: row.get(1)?,
        sequence_number: sequence.max(0) as u64,
        event_type: row.get(3)?,
        previous_state: row.get(4)?,
        current_state: row.get(5)?,
        outcome: row.get(6)?,
        detail_code: row.get(7)?,
        actor_type: row.get(8)?,
        actor_id: row.get(9)?,
        metadata: serde_json::from_str(&metadata_json).unwrap_or_else(|_| json!({})),
        event_hash: row.get(11)?,
        created_at_utc: row.get(12)?,
    })
}

fn delivery_from_row(row: &Row<'_>) -> rusqlite::Result<DeliverySummary> {
    let attempts: i64 = row.get(6)?;
    Ok(DeliverySummary {
        delivery_id: row.get(0)?,
        job_id: row.get(1)?,
        receipt_id: row.get(2)?,
        connection_id: row.get(3)?,
        state: row.get(4)?,
        payload_hash: row.get(5)?,
        attempt_count: attempts.max(0) as u32,
        next_attempt_at_utc: row.get(7)?,
        acknowledged_at_utc: row.get(8)?,
        expires_at_utc: row.get(9)?,
    })
}

fn worker_from_row(row: &Row<'_>) -> rusqlite::Result<WorkerSummary> {
    let types_json: String = row.get(3)?;
    let max_concurrent: i64 = row.get(4)?;
    let revision: i64 = row.get(6)?;
    Ok(WorkerSummary {
        worker_id: row.get(0)?,
        worker_kind: row.get(1)?,
        display_name: row.get(2)?,
        allowed_job_types: serde_json::from_str(&types_json).unwrap_or_default(),
        max_concurrent_jobs: max_concurrent.max(0) as u32,
        state: row.get(5)?,
        revision: revision.max(0) as u64,
        last_seen_at_utc: row.get(7)?,
    })
}

fn snapshot_with_connection(
    connection: &Connection,
    request: SnapshotRequest,
) -> Result<ConnectionJobSnapshot> {
    reconcile_authority(connection)?;
    let connection_id = validate_uuid(&request.connection_id, "connection ID")?;
    let exists: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_connections c JOIN wrapper_identities w ON w.wrapper_id=c.wrapper_id WHERE c.connection_id=?1 AND c.lifecycle_state IN ('active','offline','grace') AND w.state='active'",
        params![connection_id],
        |row| row.get(0),
    )?;
    ensure!(exists == 1, "active wrapper connection was not found");
    let limit = i64::from(request.limit.unwrap_or(100).clamp(1, MAX_JOBS_PER_SNAPSHOT as u32));
    let sql = format!("{JOB_RECORD_SELECT} WHERE j.connection_id=?1 ORDER BY j.created_at_utc DESC,j.job_id DESC LIMIT ?2");
    let mut statement = connection.prepare(&sql)?;
    let records = statement
        .query_map(params![connection_id, limit], job_record_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let jobs = records
        .into_iter()
        .map(|record| job_summary(connection, record))
        .collect::<Result<Vec<_>>>()?;
    let mut event_statement = connection.prepare(
        "SELECT event_id,job_id,sequence_number,event_type,previous_state,current_state,outcome,detail_code,actor_type,actor_id,metadata_json,event_hash,created_at_utc FROM wrapper_job_events WHERE connection_id=?1 AND visibility='wrapper' ORDER BY created_at_utc DESC,event_id DESC LIMIT ?2",
    )?;
    let events = event_statement
        .query_map(params![connection_id, MAX_EVENTS_PER_SNAPSHOT], read_event)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut delivery_statement = connection.prepare(
        "SELECT delivery_id,job_id,receipt_id,connection_id,state,payload_hash,attempt_count,next_attempt_at_utc,acknowledged_at_utc,expires_at_utc FROM wrapper_job_deliveries WHERE connection_id=?1 AND state IN ('pending','in_flight') ORDER BY created_at_utc,delivery_id LIMIT ?2",
    )?;
    let pending_deliveries = delivery_statement
        .query_map(params![connection_id, MAX_DELIVERIES_PER_POLL], delivery_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ConnectionJobSnapshot {
        schema: "homeserver.wrapper-jobs.v1".to_owned(),
        connection_id,
        queued_jobs: jobs.iter().filter(|job| job.state == "queued").count() as u64,
        active_jobs: jobs
            .iter()
            .filter(|job| matches!(job.state.as_str(), "leased" | "running" | "waiting"))
            .count() as u64,
        terminal_jobs: jobs
            .iter()
            .filter(|job| TERMINAL_STATES.contains(&job.state.as_str()))
            .count() as u64,
        jobs,
        events,
        pending_deliveries,
        private_inputs_exposed: false,
        private_results_exposed: false,
    })
}
