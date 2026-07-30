struct JobGrantConstraints {
    allowed_fields: Vec<String>,
    result_policy: String,
    max_result_bytes: u64,
    max_execution_seconds: u32,
    max_queued_jobs: u32,
}

pub fn submit_job(state: &AppState, request: SubmitJobRequest) -> Result<SubmittedJobResponse> {
    let connection_id = validate_uuid(&request.connection_id, "connection ID")?;
    let capability_key = validate_symbol(&request.capability_key, 120, "capability key")?;
    let operation = validate_symbol(&request.operation, 80, "operation")?;
    let job_type = validate_symbol(&request.job_type, 80, "job type")?;
    let idempotency_key = validate_idempotency_key(&request.idempotency_key)?;
    let submitted_by_type = validate_submitter_type(&request.submitted_by_type)?;
    let submitted_by_id = bounded_text(&request.submitted_by_id, 1, 160, "submitter ID")?;
    ensure!(!request.private_input.is_null(), "private job input is required");
    let private_input_text = json_text(&request.private_input)?;
    ensure!(
        (2..=MAX_PRIVATE_INPUT_BYTES).contains(&private_input_text.len()),
        "private job input exceeds the HomeServer limit"
    );
    let payload_hash = hash_text(&private_input_text);
    let scope_kind = request
        .scope_kind
        .as_deref()
        .map(|value| validate_symbol(value, 40, "scope kind"))
        .transpose()?;
    let scope_value = request
        .scope_value
        .as_deref()
        .map(|value| bounded_text(value, 1, 240, "scope value"))
        .transpose()?;
    ensure!(
        scope_kind.is_some() == scope_value.is_some(),
        "scope kind and value must be supplied together"
    );
    if capability_key == "action.propose" {
        ensure!(operation == "propose", "action authority is proposal-only");
        ensure!(
            job_type.contains("proposal"),
            "action proposal jobs must use a proposal job type"
        );
    }
    let priority = request.priority.unwrap_or(5);
    ensure!(priority <= 9, "job priority must be between zero and nine");
    let max_attempts = request.max_attempts.unwrap_or(3);
    ensure!((1..=20).contains(&max_attempts), "maximum attempts must be between 1 and 20");
    ensure!(
        (1..=10_080).contains(&request.expires_minutes),
        "job expiration must be between one minute and seven days"
    );
    let approval_id = request
        .approval_id
        .as_deref()
        .map(|value| validate_uuid(value, "approval ID"))
        .transpose()?;
    let plan_hash = request
        .plan_hash
        .as_deref()
        .map(|value| validate_sha256(value, "plan hash"))
        .transpose()?;
    ensure!(
        approval_id.is_some() == plan_hash.is_some(),
        "approval ID and plan hash must be supplied together"
    );
    let correlation_id = request
        .correlation_id
        .as_deref()
        .map(|value| bounded_text(value, 1, 160, "correlation ID"))
        .transpose()?
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let causation_id = request
        .causation_id
        .as_deref()
        .map(|value| bounded_text(value, 1, 160, "causation ID"))
        .transpose()?;
    let available_at = request
        .available_at_utc
        .as_deref()
        .map(|value| parse_utc(value, "available time"))
        .transpose()?
        .unwrap_or_else(Utc::now);
    ensure!(
        available_at <= Utc::now() + Duration::days(7),
        "job availability cannot be delayed more than seven days"
    );

    let request_document = json!({
        "schema": "homeserver.wrapper-job-request.v1",
        "connection_id": connection_id,
        "capability_key": capability_key,
        "operation": operation,
        "job_type": job_type,
        "idempotency_key": idempotency_key,
        "payload_hash": payload_hash,
        "scope_kind": scope_kind,
        "scope_value": scope_value,
        "estimated_result_bytes": request.estimated_result_bytes.unwrap_or(0),
        "estimated_token_count": request.estimated_token_count.unwrap_or(0),
        "approval_id": approval_id,
        "plan_hash": plan_hash,
        "correlation_id": correlation_id,
        "causation_id": causation_id,
        "submitted_by_type": submitted_by_type,
        "submitted_by_id": submitted_by_id,
        "priority": priority,
        "expires_minutes": request.expires_minutes,
        "max_attempts": max_attempts,
        "available_at_utc": available_at.to_rfc3339_opts(SecondsFormat::Millis, true)
    });
    let request_hash = hash_json(&request_document)?;
    let connection = state.connection()?;

    if let Some(existing) = existing_submission(&connection, &connection_id, &idempotency_key)? {
        ensure!(
            existing.request_hash == request_hash,
            "idempotency key was already used with a different request"
        );
        return Ok(SubmittedJobResponse {
            job_id: existing.job_id,
            state: existing.state,
            idempotency_key: existing.idempotency_key,
            request_hash: existing.request_hash,
            payload_hash: existing.payload_hash,
            authorization_decision_id: existing.authorization_decision_id,
            grant_id: existing.grant_id,
            grant_revision: existing.grant_revision,
            connection_authority_revision: existing.connection_authority_revision,
            result_policy: existing.result_policy,
            replayed: true,
        });
    }

    let authorization = wrapper_grants::authorize(
        &connection,
        AuthorizeRequest {
            connection_id: connection_id.clone(),
            capability_key: capability_key.clone(),
            operation: operation.clone(),
            scope_kind: scope_kind.clone(),
            scope_value: scope_value.clone(),
            result_bytes: request.estimated_result_bytes,
            token_count: request.estimated_token_count,
            approval_id: approval_id.clone(),
            plan_hash: plan_hash.clone(),
            correlation_id: Some(correlation_id.clone()),
        },
    )?;
    ensure!(
        authorization.allowed,
        "job authority was denied: {}",
        authorization.detail_code
    );
    let grant_id = authorization
        .grant_id
        .clone()
        .context("job authorization did not identify a grant")?;
    let connection_authority_revision =
        current_connection_authority_revision(&connection, &connection_id)?;
    let constraints = job_grant_constraints(
        &connection,
        &grant_id,
        scope_kind.as_deref(),
        scope_value.as_deref(),
        authorization.result_policy.as_deref(),
    )?;
    let queued: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_jobs WHERE grant_id=?1 AND state IN ('queued','leased','running','waiting')",
        params![grant_id],
        |row| row.get(0),
    )?;
    ensure!(
        queued < i64::from(constraints.max_queued_jobs),
        "grant queued-job limit exceeded"
    );
    let now = Utc::now();
    let requested_expiration = now + Duration::minutes(i64::from(request.expires_minutes));
    let grant_expiration = authorization
        .expires_at_utc
        .as_deref()
        .map(|value| parse_utc(value, "grant expiration"))
        .transpose()?
        .context("job grant expiration is unavailable")?;
    let expires_at = requested_expiration.min(grant_expiration);
    ensure!(available_at < expires_at, "job expires before it becomes available");
    let now_text = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    let available_text = available_at.to_rfc3339_opts(SecondsFormat::Millis, true);
    let expires_text = expires_at.to_rfc3339_opts(SecondsFormat::Millis, true);
    let job_id = Uuid::new_v4().to_string();
    let scope_hash = scope_kind
        .as_deref()
        .zip(scope_value.as_deref())
        .map(|(kind, value)| hash_text(&format!("{kind}:{value}")));
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO wrapper_jobs (job_id,wrapper_id,connection_id,grant_id,grant_revision,authorization_decision_id,capability_key,operation,job_type,state,priority,idempotency_key,request_hash,payload_hash,scope_kind,scope_value,scope_hash,result_policy,allowed_result_fields_json,max_result_bytes,max_execution_seconds,max_attempts,attempt_count,approval_id,plan_hash,correlation_id,causation_id,submitted_by_type,submitted_by_id,available_at_utc,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'queued',?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,0,?22,?23,?24,?25,?26,?27,?28,?29,?30,?30)",
        params![
            job_id,
            authorization.wrapper_id,
            connection_id,
            grant_id,
            authorization.grant_revision as i64,
            authorization.decision_id,
            capability_key,
            operation,
            job_type,
            i64::from(priority),
            idempotency_key,
            request_hash,
            payload_hash,
            scope_kind,
            scope_value,
            scope_hash,
            constraints.result_policy,
            serde_json::to_string(&constraints.allowed_fields)?,
            constraints.max_result_bytes as i64,
            i64::from(constraints.max_execution_seconds),
            i64::from(max_attempts),
            approval_id,
            plan_hash,
            correlation_id,
            causation_id,
            submitted_by_type,
            submitted_by_id,
            available_text,
            expires_text,
            now_text
        ],
    )?;
    transaction.execute(
        "INSERT INTO wrapper_job_inputs (job_id,private_input_json,private_input_bytes,created_at_utc) VALUES (?1,?2,?3,?4)",
        params![job_id, private_input_text, request.private_input.to_string().len() as i64, now_text],
    )?;
    transaction.execute(
        "INSERT INTO wrapper_job_authority_snapshots (job_id,connection_id,grant_id,connection_authority_revision,grant_revision,authorization_decision_id,captured_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            job_id,
            connection_id,
            grant_id,
            connection_authority_revision as i64,
            authorization.grant_revision as i64,
            authorization.decision_id,
            now_text
        ],
    )?;
    let job = job_record_by_id_tx(&transaction, &job_id)?;
    record_job_event(
        &transaction,
        &job,
        JobEventEvidence {
            event_type: "wrapper.job.submitted",
            previous_state: None,
            current_state: "queued",
            outcome: "success",
            detail_code: "authorized_and_queued",
            actor_type: &job.submitted_by_type,
            actor_id: &job.submitted_by_id,
            visibility: "wrapper",
            metadata: json!({
                "authorization_decision_id": job.authorization_decision_id,
                "grant_id": job.grant_id,
                "grant_revision": job.grant_revision,
                "connection_authority_revision": connection_authority_revision,
                "payload_hash": job.payload_hash,
                "result_policy": job.result_policy
            }),
        },
    )?;
    transaction.commit()?;
    Ok(SubmittedJobResponse {
        job_id,
        state: "queued".to_owned(),
        idempotency_key,
        request_hash,
        payload_hash,
        authorization_decision_id: authorization.decision_id,
        grant_id,
        grant_revision: authorization.grant_revision,
        connection_authority_revision,
        result_policy: constraints.result_policy,
        replayed: false,
    })
}

fn job_grant_constraints(
    connection: &Connection,
    grant_id: &str,
    scope_kind: Option<&str>,
    scope_value: Option<&str>,
    authorized_policy: Option<&str>,
) -> Result<JobGrantConstraints> {
    let (max_result_bytes, max_execution_seconds, max_queued_jobs): (i64, i64, i64) =
        connection.query_row(
            "SELECT max_result_bytes,max_execution_seconds,max_queued_jobs FROM wrapper_resource_limits WHERE grant_id=?1",
            params![grant_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    let (allowed_fields, scoped_policy) = if let Some((kind, value)) = scope_kind.zip(scope_value) {
        let (fields_json, policy): (String, String) = connection.query_row(
            "SELECT allowed_fields_json,result_policy FROM wrapper_dataset_scopes WHERE grant_id=?1 AND scope_kind=?2 AND scope_value=?3 AND state='active'",
            params![grant_id, kind, value],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        (
            serde_json::from_str::<Vec<String>>(&fields_json).unwrap_or_default(),
            policy,
        )
    } else {
        (Vec::new(), authorized_policy.unwrap_or("receipt_only").to_owned())
    };
    let result_policy = authorized_policy.unwrap_or(&scoped_policy).to_owned();
    ensure!(
        result_policy == scoped_policy || scope_kind.is_none(),
        "authorized result policy does not match the scope contract"
    );
    Ok(JobGrantConstraints {
        allowed_fields,
        result_policy,
        max_result_bytes: max_result_bytes.max(1024) as u64,
        max_execution_seconds: max_execution_seconds.max(1) as u32,
        max_queued_jobs: max_queued_jobs.max(1) as u32,
    })
}

fn existing_submission(
    connection: &Connection,
    connection_id: &str,
    idempotency_key: &str,
) -> Result<Option<JobRecord>> {
    connection
        .query_row(
            JOB_RECORD_QUERY_BY_IDEMPOTENCY,
            params![connection_id, idempotency_key],
            job_record_from_row,
        )
        .optional()
        .map_err(Into::into)
}
