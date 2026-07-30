fn project_safe_result(job: &JobRecord, private_result: &Value) -> Result<Value> {
    match job.result_policy.as_str() {
        "receipt_only" => Ok(json!({})),
        "metadata_only" => project_selected_fields(
            private_result,
            &effective_fields(
                &job.allowed_result_fields,
                &["status", "count", "duration_ms", "model", "result_code", "completed_at_utc"],
            ),
            ProjectionMode::Metadata,
        ),
        "aggregate_only" => project_selected_fields(
            private_result,
            &job.allowed_result_fields,
            ProjectionMode::Aggregate,
        ),
        "proposal_only" => project_selected_fields(
            private_result,
            &effective_fields(
                &job.allowed_result_fields,
                &["title", "summary", "proposed_action", "requires_approval"],
            ),
            ProjectionMode::Proposal,
        ),
        "safe_result" => {
            ensure!(
                !job.allowed_result_fields.is_empty(),
                "safe-result jobs require an explicit result-field allowlist"
            );
            project_selected_fields(
                private_result,
                &job.allowed_result_fields,
                ProjectionMode::Safe,
            )
        }
        _ => bail!("job result policy is unsupported"),
    }
}

#[derive(Clone, Copy)]
enum ProjectionMode {
    Safe,
    Metadata,
    Aggregate,
    Proposal,
}

fn effective_fields(configured: &[String], defaults: &[&str]) -> Vec<String> {
    if configured.is_empty() {
        defaults.iter().map(|value| (*value).to_owned()).collect()
    } else {
        configured.to_vec()
    }
}

fn project_selected_fields(
    private_result: &Value,
    allowed_fields: &[String],
    mode: ProjectionMode,
) -> Result<Value> {
    let source = private_result
        .as_object()
        .context("job result must be a JSON object")?;
    let allowed = allowed_fields.iter().cloned().collect::<BTreeSet<_>>();
    let mut output = Map::new();
    for field in allowed {
        ensure!(!is_forbidden_result_key(&field), "result field is private or forbidden");
        let Some(value) = source.get(&field) else {
            continue;
        };
        let projected = match mode {
            ProjectionMode::Aggregate => project_aggregate_value(value)?,
            ProjectionMode::Proposal if field == "proposed_action" => {
                project_proposed_action(value)?
            }
            _ => sanitize_result_value(value, 0)?,
        };
        output.insert(field, projected);
    }
    Ok(Value::Object(output))
}

fn project_aggregate_value(value: &Value) -> Result<Value> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
        Value::Array(items) => {
            ensure!(items.len() <= 100, "aggregate result array is too large");
            items.iter().map(project_aggregate_value).collect::<Result<Vec<_>>>().map(Value::Array)
        }
        Value::Object(map) => {
            ensure!(map.len() <= 100, "aggregate result object is too large");
            let mut output = Map::new();
            for (key, value) in map {
                ensure!(!is_forbidden_result_key(key), "aggregate result contains a forbidden key");
                output.insert(key.clone(), project_aggregate_value(value)?);
            }
            Ok(Value::Object(output))
        }
        Value::String(_) => bail!("aggregate-only results cannot contain strings"),
    }
}

fn project_proposed_action(value: &Value) -> Result<Value> {
    let source = value
        .as_object()
        .context("proposed action must be a JSON object")?;
    let allowed = ["type", "summary", "target_type", "requires_approval"];
    let mut output = Map::new();
    for field in allowed {
        if let Some(value) = source.get(field) {
            output.insert(field.to_owned(), sanitize_result_value(value, 0)?);
        }
    }
    ensure!(
        output.get("requires_approval") == Some(&Value::Bool(true)),
        "proposed actions must remain approval-gated"
    );
    Ok(Value::Object(output))
}

fn sanitize_result_value(value: &Value, depth: usize) -> Result<Value> {
    ensure!(depth <= 6, "safe result exceeds the maximum nesting depth");
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
        Value::String(text) => {
            ensure!(text.chars().count() <= 4_000, "safe result string is too long");
            ensure!(!text.chars().any(char::is_control), "safe result contains control characters");
            let lowered = text.to_ascii_lowercase();
            ensure!(
                !lowered.contains("-----begin private key")
                    && !lowered.contains("api_key=")
                    && !lowered.contains("authorization: bearer")
                    && !lowered.contains("bearer eyj"),
                "safe result appears to contain credential material"
            );
            Ok(Value::String(text.clone()))
        }
        Value::Array(items) => {
            ensure!(items.len() <= 100, "safe result array is too large");
            items
                .iter()
                .map(|item| sanitize_result_value(item, depth + 1))
                .collect::<Result<Vec<_>>>()
                .map(Value::Array)
        }
        Value::Object(map) => {
            ensure!(map.len() <= 100, "safe result object is too large");
            let mut output = Map::new();
            for (key, value) in map {
                ensure!(!is_forbidden_result_key(key), "safe result contains a private field");
                output.insert(key.clone(), sanitize_result_value(value, depth + 1)?);
            }
            Ok(Value::Object(output))
        }
    }
}

fn is_forbidden_result_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace('-', "_");
    [
        "source",
        "source_id",
        "source_text",
        "raw",
        "raw_text",
        "document",
        "full_document",
        "full_text",
        "prompt",
        "system_prompt",
        "credential",
        "credentials",
        "secret",
        "token",
        "api_key",
        "memory",
        "private",
        "private_data",
        "conversation",
        "embedding",
        "file_path",
        "local_path",
        "email_body",
    ]
    .iter()
    .any(|forbidden| normalized == *forbidden || normalized.ends_with(&format!("_{forbidden}")))
}

fn safe_provenance_summary(
    source_count: u32,
    source_types: &[String],
    evidence_hash: Option<&str>,
) -> Result<Value> {
    ensure!(source_count <= 100_000, "source count exceeds the supported limit");
    ensure!(source_types.len() <= 32, "too many provenance source types");
    let mut types = BTreeSet::new();
    for source_type in source_types {
        types.insert(validate_symbol(source_type, 60, "source type")?);
    }
    let evidence_hash = evidence_hash
        .map(|value| validate_sha256(value, "evidence hash"))
        .transpose()?;
    Ok(json!({
        "source_count": source_count,
        "source_types": types.into_iter().collect::<Vec<_>>(),
        "evidence_hash": evidence_hash,
        "source_identifiers_included": false,
        "private_source_content_included": false
    }))
}
