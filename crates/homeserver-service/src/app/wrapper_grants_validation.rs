fn normalize_scopes(scopes: Vec<ScopeInput>, rule: &CapabilityRule) -> Result<Vec<ScopeInput>> {
    ensure!(
        scopes.len() <= MAX_SCOPES_PER_GRANT,
        "too many grant scopes"
    );
    if rule.requires_scope {
        ensure!(!scopes.is_empty(), "capability requires at least one exact scope");
    }
    let mut unique = BTreeSet::new();
    let mut normalized = Vec::with_capacity(scopes.len());
    for scope in scopes {
        let scope_kind = validate_scope_kind(&scope.scope_kind)?;
        let scope_value = validate_scope_value(&scope.scope_value)?;
        ensure!(
            unique.insert(format!("{scope_kind}:{scope_value}")),
            "duplicate grant scope"
        );
        ensure!(
            scope.allowed_fields.len() <= MAX_ALLOWED_FIELDS,
            "too many allowed fields"
        );
        let mut fields = BTreeSet::new();
        for field in scope.allowed_fields {
            let field = bounded_identifier(&field, 1, 120, "allowed field")?;
            ensure!(field != "*", "wildcard fields are forbidden");
            fields.insert(field);
        }
        ensure!(scope.filter.is_object(), "scope filter must be a JSON object");
        let result_policy = validate_result_policy(
            scope
                .result_policy
                .as_deref()
                .unwrap_or(rule.result_mode.as_str()),
        )?;
        normalized.push(ScopeInput {
            scope_kind,
            scope_value,
            allowed_fields: fields.into_iter().collect(),
            filter: scope.filter,
            result_policy: Some(result_policy),
        });
    }
    Ok(normalized)
}

fn normalize_limits(input: Option<ResourceLimitsInput>, risk_tier: &str) -> Result<ResourceLimits> {
    let ceiling = match risk_tier {
        "low" => ResourceLimits {
            requests_per_minute: 120,
            max_result_bytes: 262_144,
            max_daily_tokens: 200_000,
            max_concurrent_jobs: 4,
            max_queued_jobs: 50,
            max_execution_seconds: 300,
        },
        "medium" => ResourceLimits {
            requests_per_minute: 60,
            max_result_bytes: 131_072,
            max_daily_tokens: 100_000,
            max_concurrent_jobs: 2,
            max_queued_jobs: 25,
            max_execution_seconds: 180,
        },
        "high" => ResourceLimits {
            requests_per_minute: 30,
            max_result_bytes: 65_536,
            max_daily_tokens: 25_000,
            max_concurrent_jobs: 1,
            max_queued_jobs: 10,
            max_execution_seconds: 120,
        },
        "critical" => ResourceLimits {
            requests_per_minute: 10,
            max_result_bytes: 32_768,
            max_daily_tokens: 0,
            max_concurrent_jobs: 0,
            max_queued_jobs: 5,
            max_execution_seconds: 60,
        },
        _ => bail!("unknown capability risk tier"),
    };
    let input = input.unwrap_or(ResourceLimitsInput {
        requests_per_minute: None,
        max_result_bytes: None,
        max_daily_tokens: None,
        max_concurrent_jobs: None,
        max_queued_jobs: None,
        max_execution_seconds: None,
    });
    let limits = ResourceLimits {
        requests_per_minute: input
            .requests_per_minute
            .unwrap_or(ceiling.requests_per_minute),
        max_result_bytes: input.max_result_bytes.unwrap_or(ceiling.max_result_bytes),
        max_daily_tokens: input.max_daily_tokens.unwrap_or(ceiling.max_daily_tokens),
        max_concurrent_jobs: input
            .max_concurrent_jobs
            .unwrap_or(ceiling.max_concurrent_jobs),
        max_queued_jobs: input.max_queued_jobs.unwrap_or(ceiling.max_queued_jobs),
        max_execution_seconds: input
            .max_execution_seconds
            .unwrap_or(ceiling.max_execution_seconds),
    };
    ensure!(
        limits.requests_per_minute >= 1
            && limits.requests_per_minute <= ceiling.requests_per_minute,
        "requests-per-minute limit exceeds the capability ceiling"
    );
    ensure!(
        (1024..=ceiling.max_result_bytes).contains(&limits.max_result_bytes),
        "result-size limit exceeds the capability ceiling"
    );
    ensure!(
        limits.max_daily_tokens <= ceiling.max_daily_tokens,
        "daily token limit exceeds the capability ceiling"
    );
    ensure!(
        limits.max_concurrent_jobs <= ceiling.max_concurrent_jobs,
        "concurrent-job limit exceeds the capability ceiling"
    );
    ensure!(
        limits.max_queued_jobs <= ceiling.max_queued_jobs,
        "queued-job limit exceeds the capability ceiling"
    );
    ensure!(
        limits.max_execution_seconds >= 1
            && limits.max_execution_seconds <= ceiling.max_execution_seconds,
        "execution-time limit exceeds the capability ceiling"
    );
    Ok(limits)
}

fn resolve_approval_mode(requested: Option<&str>, rule: &CapabilityRule) -> Result<String> {
    let requested = requested
        .unwrap_or(&rule.default_approval_mode)
        .trim()
        .to_ascii_lowercase();
    ensure!(
        matches!(requested.as_str(), "none" | "explicit" | "per_request"),
        "approval mode is invalid"
    );
    let rank = |value: &str| match value {
        "none" => 0,
        "explicit" => 1,
        "per_request" => 2,
        _ => -1,
    };
    ensure!(
        rank(&requested) >= rank(&rule.default_approval_mode),
        "approval mode cannot weaken the capability default"
    );
    Ok(requested)
}

fn validated_expiration(
    now: DateTime<Utc>,
    expires_minutes: u32,
    risk_tier: &str,
) -> Result<DateTime<Utc>> {
    let maximum = match risk_tier {
        "critical" => 24 * 60,
        "high" => 30 * 24 * 60,
        "medium" => 90 * 24 * 60,
        "low" => 365 * 24 * 60,
        _ => bail!("unknown risk tier"),
    };
    ensure!(
        (5..=maximum).contains(&expires_minutes),
        "grant expiration is outside the allowed risk-tier window"
    );
    Ok(now + Duration::minutes(i64::from(expires_minutes)))
}

fn normalize_operations(operations: Vec<String>) -> Result<Vec<String>> {
    ensure!(!operations.is_empty(), "grant requires at least one operation");
    ensure!(operations.len() <= MAX_OPERATIONS, "too many grant operations");
    let mut values = BTreeSet::new();
    for operation in operations {
        values.insert(validate_operation(&operation)?);
    }
    Ok(values.into_iter().collect())
}

fn ensure_operation_subset(requested: &[String], allowed: &[String]) -> Result<()> {
    ensure!(
        requested.iter().all(|operation| allowed.contains(operation)),
        "grant requests an operation outside the capability catalog"
    );
    Ok(())
}

fn validate_capability_key(value: &str) -> Result<String> {
    let value = bounded_identifier(value, 3, 120, "capability key")?;
    ensure!(
        !matches!(
            value.as_str(),
            "admin" | "knowledge.all" | "tools.all" | "agent.execute_any" | "cross_wrapper.read"
        ) && !value.ends_with(".all"),
        "broad or administrative capability keys are forbidden"
    );
    Ok(value)
}

fn validate_operation(value: &str) -> Result<String> {
    bounded_identifier(value, 2, 40, "operation")
}

fn validate_scope_kind(value: &str) -> Result<String> {
    let value = bounded_identifier(value, 3, 40, "scope kind")?;
    ensure!(
        matches!(
            value.as_str(),
            "dataset" | "collection" | "record" | "tag" | "resource"
        ),
        "scope kind is invalid"
    );
    Ok(value)
}

fn validate_scope_value(value: &str) -> Result<String> {
    let value = bounded_text(value, 1, 240, "scope value")?;
    let lowered = value.to_ascii_lowercase();
    ensure!(
        !matches!(lowered.as_str(), "*" | "all" | "any" | "everything"),
        "unscoped or wildcard authority is forbidden"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "scope value contains control characters"
    );
    Ok(value)
}

fn validate_result_policy(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(
        matches!(
            value.as_str(),
            "safe_result"
                | "metadata_only"
                | "aggregate_only"
                | "proposal_only"
                | "receipt_only"
        ),
        "result policy is invalid"
    );
    Ok(value)
}

fn validate_uuid(value: &str, label: &str) -> Result<String> {
    let value = value.trim();
    Uuid::parse_str(value).with_context(|| format!("{label} is not a valid UUID"))?;
    Ok(value.to_owned())
}

fn validate_sha256(value: &str, label: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(
        value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()),
        "{label} must be a lowercase SHA-256 digest"
    );
    Ok(value)
}

fn bounded_identifier(value: &str, minimum: usize, maximum: usize, label: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(
        (minimum..=maximum).contains(&value.len()),
        "{label} has an invalid length"
    );
    ensure!(
        value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        }),
        "{label} contains unsupported characters"
    );
    Ok(value)
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

fn hash_json(value: &Value) -> Result<String> {
    Ok(hash_text(&serde_json::to_string(value)?))
}

fn hash_text(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn empty_object() -> Value {
    json!({})
}

fn now_utc() -> String {
    timestamp(Utc::now())
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn api_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    tracing::warn!(?error, code, "wrapper grant request rejected");
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string(),
        }),
    )
}

