pub fn register_worker(
    connection: &Connection,
    request: RegisterWorkerRequest,
) -> Result<WorkerSummary> {
    let worker_kind = validate_worker_kind(&request.worker_kind)?;
    let display_name = bounded_text(&request.display_name, 1, 120, "worker display name")?;
    ensure!(
        (1..=32).contains(&request.max_concurrent_jobs),
        "worker concurrency must be between 1 and 32"
    );
    ensure!(
        !request.allowed_job_types.is_empty() && request.allowed_job_types.len() <= 64,
        "worker must declare between 1 and 64 job types"
    );
    let mut allowed_types = BTreeSet::new();
    for job_type in request.allowed_job_types {
        allowed_types.insert(validate_symbol(&job_type, 80, "job type")?);
    }
    let worker_id = Uuid::new_v4().to_string();
    let now = now_utc();
    connection.execute(
        "INSERT INTO wrapper_job_workers (worker_id,worker_kind,display_name,allowed_job_types_json,max_concurrent_jobs,state,revision,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,'active',1,?6,?6)",
        params![
            worker_id,
            worker_kind,
            display_name,
            serde_json::to_string(&allowed_types.iter().cloned().collect::<Vec<_>>())?,
            i64::from(request.max_concurrent_jobs),
            now
        ],
    )?;
    read_worker(connection, &worker_id)
}

pub fn claim_jobs(connection: &Connection, request: ClaimJobsRequest) -> Result<ClaimJobsResponse> {
    reconcile_authority(connection)?;
    let worker_id = validate_uuid(&request.worker_id, "worker ID")?;
    let worker = read_worker(connection, &worker_id)?;
    ensure!(worker.state == "active", "worker is not active");
    let active_leases: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_jobs WHERE lease_owner_id=?1 AND state IN ('leased','running') AND lease_expires_at_utc>?2",
        params![worker_id, now_utc()],
        |row| row.get(0),
    )?;
    let available_slots = i64::from(worker.max_concurrent_jobs).saturating_sub(active_leases);
    if available_slots <= 0 {
        return Ok(ClaimJobsResponse {
            worker,
            jobs: Vec::new(),
        });
    }
    let requested_limit = i64::from(request.limit.unwrap_or(4)).clamp(1, MAX_WORKER_CLAIM);
    let limit = requested_limit.min(available_slots);
    let sql = format!(
        "{JOB_RECORD_SELECT} WHERE j.state='queued' AND j.available_at_utc<=?1 AND j.expires_at_utc>?1 ORDER BY j.priority DESC,j.available_at_utc,j.created_at_utc,j.job_id LIMIT 100"
    );
    let mut statement = connection.prepare(&sql)?;
    let candidates = statement
        .query_map(params![now_utc()], job_record_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let transaction = connection.unchecked_transaction()?;
    let mut claimed = Vec::new();
    for candidate in candidates {
        if claimed.len() >= limit as usize {
            break;
        }
        if !worker
            .allowed_job_types
            .iter()
            .any(|job_type| job_type == &candidate.job_type)
        {
            continue;
        }
        if !authority_is_current_tx(&transaction, &candidate)? {
            cancel_for_authority_tx(&transaction, &candidate, "authority_changed")?;
            continue;
        }
        let max_concurrent: i64 = transaction.query_row(
            "SELECT max_concurrent_jobs FROM wrapper_resource_limits WHERE grant_id=?1",
            params![candidate.grant_id],
            |row| row.get(0),
        )?;
        let grant_active: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM wrapper_jobs WHERE grant_id=?1 AND state IN ('leased','running') AND lease_expires_at_utc>?2",
            params![candidate.grant_id, now_utc()],
            |row| row.get(0),
        )?;
        if grant_active >= max_concurrent {
            continue;
        }
        let lease_token = random_token();
        let lease_token_hash = hash_text(&lease_token);
        let now = Utc::now();
        let lease_seconds = i64::from(candidate.max_execution_seconds.min(300).max(30));
        let lease_expires = (now + Duration::seconds(lease_seconds))
            .min(parse_utc(&candidate.expires_at_utc, "job expiration")?)
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let changed = transaction.execute(
            "UPDATE wrapper_jobs SET state='leased',attempt_count=attempt_count+1,lease_owner_id=?1,lease_token_hash=?2,lease_expires_at_utc=?3,updated_at_utc=?4 WHERE job_id=?5 AND state='queued'",
            params![worker_id, lease_token_hash, lease_expires, now_utc(), candidate.job_id],
        )?;
        if changed != 1 {
            continue;
        }
        let leased_job = job_record_by_id_tx(&transaction, &candidate.job_id)?;
        record_job_event(
            &transaction,
            &leased_job,
            JobEventEvidence {
                event_type: "wrapper.job.leased",
                previous_state: Some("queued"),
                current_state: "leased",
                outcome: "success",
                detail_code: "worker_lease_issued",
                actor_type: "worker",
                actor_id: &worker_id,
                visibility: "internal",
                metadata: json!({
                    "worker_kind": worker.worker_kind,
                    "lease_expires_at_utc": lease_expires,
                    "attempt_count": leased_job.attempt_count
                }),
            },
        )?;
        let private_input_json: String = transaction.query_row(
            "SELECT private_input_json FROM wrapper_job_inputs WHERE job_id=?1",
            params![candidate.job_id],
            |row| row.get(0),
        )?;
        claimed.push((
            candidate.job_id,
            lease_token,
            serde_json::from_str::<Value>(&private_input_json).unwrap_or_else(|_| json!({})),
        ));
    }
    transaction.execute(
        "UPDATE wrapper_job_workers SET last_seen_at_utc=?1,updated_at_utc=?1 WHERE worker_id=?2",
        params![now_utc(), worker_id],
    )?;
    transaction.commit()?;
    let jobs = claimed
        .into_iter()
        .map(|(job_id, lease_token, private_input)| {
            let record = job_record_by_id(connection, &job_id)?;
            Ok(LeasedJob {
                job: job_summary(connection, record)?,
                lease_token,
                private_input,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ClaimJobsResponse { worker, jobs })
}

pub fn start_job(connection: &Connection, request: WorkerLeaseRequest) -> Result<JobSummary> {
    reconcile_authority(connection)?;
    let worker_id = validate_uuid(&request.worker_id, "worker ID")?;
    let job_id = validate_uuid(&request.job_id, "job ID")?;
    let lease_token = bounded_text(&request.lease_token, 32, 128, "lease token")?;
    let transaction = connection.unchecked_transaction()?;
    let job = validate_worker_lease_tx(&transaction, &worker_id, &job_id, &lease_token, &["leased"])?;
    ensure!(authority_is_current_tx(&transaction, &job)?, "job authority changed");
    let now = now_utc();
    transaction.execute(
        "UPDATE wrapper_jobs SET state='running',started_at_utc=COALESCE(started_at_utc,?1),updated_at_utc=?1 WHERE job_id=?2 AND state='leased'",
        params![now, job_id],
    )?;
    let running = job_record_by_id_tx(&transaction, &job_id)?;
    record_job_event(
        &transaction,
        &running,
        JobEventEvidence {
            event_type: "wrapper.job.started",
            previous_state: Some("leased"),
            current_state: "running",
            outcome: "success",
            detail_code: "worker_started",
            actor_type: "worker",
            actor_id: &worker_id,
            visibility: "wrapper",
            metadata: json!({"attempt_count": running.attempt_count}),
        },
    )?;
    transaction.execute(
        "UPDATE wrapper_job_workers SET last_seen_at_utc=?1,updated_at_utc=?1 WHERE worker_id=?2",
        params![now, worker_id],
    )?;
    transaction.commit()?;
    job_summary(connection, job_record_by_id(connection, &job_id)?)
}

pub fn heartbeat_job(connection: &Connection, request: WorkerLeaseRequest) -> Result<JobSummary> {
    reconcile_authority(connection)?;
    let worker_id = validate_uuid(&request.worker_id, "worker ID")?;
    let job_id = validate_uuid(&request.job_id, "job ID")?;
    let lease_token = bounded_text(&request.lease_token, 32, 128, "lease token")?;
    let transaction = connection.unchecked_transaction()?;
    let job = validate_worker_lease_tx(
        &transaction,
        &worker_id,
        &job_id,
        &lease_token,
        &["leased", "running"],
    )?;
    ensure!(authority_is_current_tx(&transaction, &job)?, "job authority changed");
    let now = Utc::now();
    let lease_expires = (now + Duration::seconds(60))
        .min(parse_utc(&job.expires_at_utc, "job expiration")?)
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    transaction.execute(
        "UPDATE wrapper_jobs SET lease_expires_at_utc=?1,updated_at_utc=?2 WHERE job_id=?3",
        params![lease_expires, now_utc(), job_id],
    )?;
    transaction.execute(
        "UPDATE wrapper_job_workers SET last_seen_at_utc=?1,updated_at_utc=?1 WHERE worker_id=?2",
        params![now_utc(), worker_id],
    )?;
    transaction.commit()?;
    job_summary(connection, job_record_by_id(connection, &job_id)?)
}

fn read_worker(connection: &Connection, worker_id: &str) -> Result<WorkerSummary> {
    connection
        .query_row(
            "SELECT worker_id,worker_kind,display_name,allowed_job_types_json,max_concurrent_jobs,state,revision,last_seen_at_utc FROM wrapper_job_workers WHERE worker_id=?1",
            params![worker_id],
            worker_from_row,
        )
        .context("wrapper job worker was not found")
}

fn validate_worker_lease_tx(
    transaction: &Transaction<'_>,
    worker_id: &str,
    job_id: &str,
    lease_token: &str,
    allowed_states: &[&str],
) -> Result<JobRecord> {
    let worker_state: String = transaction.query_row(
        "SELECT state FROM wrapper_job_workers WHERE worker_id=?1",
        params![worker_id],
        |row| row.get(0),
    )?;
    ensure!(worker_state == "active", "worker is not active");
    let job = job_record_by_id_tx(transaction, job_id)?;
    ensure!(allowed_states.contains(&job.state.as_str()), "job is not in a lease-valid state");
    ensure!(
        job.lease_owner_id.as_deref() == Some(worker_id),
        "job lease belongs to a different worker"
    );
    ensure!(
        job.lease_token_hash.as_deref() == Some(hash_text(lease_token).as_str()),
        "job lease token is invalid"
    );
    let lease_expires = job
        .lease_expires_at_utc
        .as_deref()
        .context("job lease expiration is unavailable")?;
    ensure!(
        parse_utc(lease_expires, "lease expiration")? > Utc::now(),
        "job lease expired"
    );
    Ok(job)
}
