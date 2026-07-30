struct JobEventEvidence<'a> {
    event_type: &'a str,
    previous_state: Option<&'a str>,
    current_state: &'a str,
    outcome: &'a str,
    detail_code: &'a str,
    actor_type: &'a str,
    actor_id: &'a str,
    visibility: &'a str,
    metadata: Value,
}

fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_utc(value: &str, label: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("{label} is not a valid UTC timestamp"))
        .map(|value| value.with_timezone(&Utc))
}

fn bounded_text(value: &str, minimum: usize, maximum: usize, label: &str) -> Result<String> {
    let value = value.trim();
    ensure!(
        (minimum..=maximum).contains(&value.chars().count()),
        "{label} must contain between {minimum} and {maximum} characters"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "{label} contains control characters"
    );
    Ok(value.to_owned())
}

fn validate_uuid(value: &str, label: &str) -> Result<String> {
    let value = bounded_text(value, 32, 40, label)?;
    Uuid::parse_str(&value).with_context(|| format!("{label} is invalid"))?;
    Ok(value)
}

fn validate_symbol(value: &str, maximum: usize, label: &str) -> Result<String> {
    let value = bounded_text(value, 1, maximum, label)?.to_ascii_lowercase();
    ensure!(
        value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ':')
        }),
        "{label} contains unsupported characters"
    );
    ensure!(
        !value.starts_with('.') && !value.ends_with('.') && !value.contains(".."),
        "{label} is not normalized"
    );
    Ok(value)
}

fn validate_idempotency_key(value: &str) -> Result<String> {
    let value = bounded_text(value, 8, 160, "idempotency key")?;
    ensure!(
        value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':' | '.')
        }),
        "idempotency key contains unsupported characters"
    );
    Ok(value)
}

fn validate_sha256(value: &str, label: &str) -> Result<String> {
    let value = bounded_text(value, 64, 64, label)?.to_ascii_lowercase();
    ensure!(
        value.chars().all(|character| character.is_ascii_hexdigit()),
        "{label} must be a SHA-256 hex digest"
    );
    Ok(value)
}

fn validate_actor_type(value: &str) -> Result<String> {
    let value = validate_symbol(value, 40, "actor type")?;
    ensure!(
        matches!(value.as_str(), "wrapper" | "local_user" | "worker" | "agent" | "system"),
        "actor type is unsupported"
    );
    Ok(value)
}

fn validate_submitter_type(value: &str) -> Result<String> {
    let value = validate_symbol(value, 40, "submitter type")?;
    ensure!(
        SUBMITTER_TYPES.contains(&value.as_str()),
        "submitter type is unsupported"
    );
    Ok(value)
}

fn validate_worker_kind(value: &str) -> Result<String> {
    let value = validate_symbol(value, 40, "worker kind")?;
    ensure!(WORKER_KINDS.contains(&value.as_str()), "worker kind is unsupported");
    Ok(value)
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut ordered = Map::new();
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if let Some(value) = map.get(key) {
                    ordered.insert(key.clone(), canonical_json(value));
                }
            }
            Value::Object(ordered)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        _ => value.clone(),
    }
}

fn json_text(value: &Value) -> Result<String> {
    serde_json::to_string(&canonical_json(value)).map_err(Into::into)
}

fn json_bytes(value: &Value) -> Result<usize> {
    Ok(json_text(value)?.len())
}

fn hash_text(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn hash_json(value: &Value) -> Result<String> {
    Ok(hash_text(&json_text(value)?))
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn next_event_sequence(transaction: &Transaction<'_>, job_id: &str) -> Result<u64> {
    let current: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence_number),0) FROM wrapper_job_events WHERE job_id=?1",
        params![job_id],
        |row| row.get(0),
    )?;
    Ok(current.max(0) as u64 + 1)
}

fn record_job_event(
    transaction: &Transaction<'_>,
    job: &JobRecord,
    evidence: JobEventEvidence<'_>,
) -> Result<String> {
    let sequence = next_event_sequence(transaction, &job.job_id)?;
    let event_id = Uuid::new_v4().to_string();
    let created_at = now_utc();
    let event_document = json!({
        "event_id": event_id,
        "job_id": job.job_id,
        "wrapper_id": job.wrapper_id,
        "connection_id": job.connection_id,
        "sequence_number": sequence,
        "event_type": evidence.event_type,
        "previous_state": evidence.previous_state,
        "current_state": evidence.current_state,
        "outcome": evidence.outcome,
        "detail_code": evidence.detail_code,
        "actor_type": evidence.actor_type,
        "actor_id": evidence.actor_id,
        "visibility": evidence.visibility,
        "metadata": evidence.metadata,
        "created_at_utc": created_at
    });
    let event_hash = hash_json(&event_document)?;
    transaction.execute(
        "INSERT INTO wrapper_job_events (event_id,job_id,wrapper_id,connection_id,sequence_number,event_type,previous_state,current_state,outcome,detail_code,actor_type,actor_id,visibility,metadata_json,event_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            event_id,
            job.job_id,
            job.wrapper_id,
            job.connection_id,
            sequence as i64,
            evidence.event_type,
            evidence.previous_state,
            evidence.current_state,
            evidence.outcome,
            evidence.detail_code,
            evidence.actor_type,
            evidence.actor_id,
            evidence.visibility,
            json_text(&evidence.metadata)?,
            event_hash,
            created_at
        ],
    )?;
    Ok(event_hash)
}

fn current_connection_authority_revision(connection: &Connection, connection_id: &str) -> Result<u64> {
    let revision: i64 = connection.query_row(
        "SELECT grant_revision FROM wrapper_connections WHERE connection_id=?1 AND lifecycle_state IN ('active','offline','grace')",
        params![connection_id],
        |row| row.get(0),
    )?;
    Ok(revision.max(0) as u64)
}

fn api_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    let message = format!("{error:#}");
    let lowered = message.to_ascii_lowercase();
    let status = if lowered.contains("not found") || lowered.contains("was not found") {
        StatusCode::NOT_FOUND
    } else if lowered.contains("expired")
        || lowered.contains("revoked")
        || lowered.contains("denied")
        || lowered.contains("authority")
        || lowered.contains("confirmation")
        || lowered.contains("lease")
    {
        StatusCode::FORBIDDEN
    } else if lowered.contains("idempotency")
        || lowered.contains("state")
        || lowered.contains("already")
        || lowered.contains("conflict")
    {
        StatusCode::CONFLICT
    } else {
        StatusCode::BAD_REQUEST
    };
    (
        status,
        Json(ApiError {
            ok: false,
            error: code,
            message,
        }),
    )
}
