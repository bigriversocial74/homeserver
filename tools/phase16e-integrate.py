from pathlib import Path

def patch(path, old, new, count=1):
    p=Path(path); text=p.read_text(encoding='utf-8')
    if old not in text:
        raise SystemExit(f'patch marker not found in {path}: {old[:80]!r}')
    p.write_text(text.replace(old,new,count),encoding='utf-8')

p=Path('crates/homeserver-service/src/app/wrapper_privacy.rs')
text=p.read_text(encoding='utf-8')
text=text.replace('pub destination_specific_aliases: bool, pub fail_closed: bool,','pub destination_specific_aliases: bool, pub fail_closed: bool, pub pairing_implies_private_authority: bool,')
text=text.replace('destination_specific_aliases:true,fail_closed:true','destination_specific_aliases:true,fail_closed:true,pairing_implies_private_authority:false')
p.write_text(text,encoding='utf-8')

patch('crates/homeserver-service/src/app.rs',
    '#[path = "app/wrapper_agents.rs"]\nmod wrapper_agents;\n',
    '#[path = "app/wrapper_agents.rs"]\nmod wrapper_agents;\n\n#[path = "app/wrapper_privacy.rs"]\nmod wrapper_privacy;\n')
patch('crates/homeserver-service/src/app.rs',
    '    knowledge_vault::initialize(&connection, &config)?;\n    document_extraction::initialize(&connection)?;',
    '    knowledge_vault::initialize(&connection, &config)?;\n    wrapper_privacy::initialize(&connection)?;\n    wrapper_privacy::maintain_history(&connection)?;\n    document_extraction::initialize(&connection)?;')
patch('crates/homeserver-service/src/app.rs',
    '            .merge(wrapper_agents::router(state.clone()))\n            .merge(knowledge_vault::router(state.clone()))',
    '            .merge(wrapper_agents::router(state.clone()))\n            .merge(wrapper_privacy::router(state.clone()))\n            .merge(knowledge_vault::router(state.clone()))')
patch('crates/homeserver-service/src/app/wrapper_jobs.rs',
    'use super::wrapper_grants::{self, AuthorizeRequest};',
    'use super::wrapper_grants::{self, AuthorizeRequest};\nuse super::wrapper_privacy;')
patch('crates/homeserver-service/src/app/wrapper_jobs.rs',
    '    pub private_input: Value,\n    pub scope_kind: Option<String>,',
    '    pub private_input: Value,\n    pub private_selector_id: Option<String>,\n    pub purpose: Option<String>,\n    pub output_schema: Option<String>,\n    pub remote_model_provider: Option<String>,\n    pub scope_kind: Option<String>,')

patch('crates/homeserver-service/src/app/wrapper_jobs_submit.rs',
    '    let connection_authority_revision =\n        current_connection_authority_revision(&connection, &connection_id)?;',
    '    let privacy_binding = wrapper_privacy::validate_job_privacy_submission(\n        &connection,\n        &connection_id,\n        &grant_id,\n        authorization.grant_revision,\n        &capability_key,\n        &operation,\n        &submitted_by_type,\n        &submitted_by_id,\n        request.private_selector_id.as_deref(),\n        request.purpose.as_deref(),\n        request.output_schema.as_deref(),\n        request.remote_model_provider.as_deref(),\n    )?;\n    let connection_authority_revision =\n        current_connection_authority_revision(&connection, &connection_id)?;')
patch('crates/homeserver-service/src/app/wrapper_jobs_submit.rs',
    '    if let Some(binding) = agent_binding.as_ref() {\n        wrapper_agents::bind_agent_job_tx(&transaction, &job_id, binding)?;\n    }\n    let job = job_record_by_id_tx(&transaction, &job_id)?;',
    '    if let Some(binding) = agent_binding.as_ref() {\n        wrapper_agents::bind_agent_job_tx(&transaction, &job_id, binding)?;\n    }\n    if let Some(binding) = privacy_binding.as_ref() {\n        wrapper_privacy::bind_job_privacy_tx(&transaction, &job_id, binding)?;\n    }\n    let job = job_record_by_id_tx(&transaction, &job_id)?;')

p=Path('crates/homeserver-service/src/app/wrapper_jobs_completion.rs')
text=p.read_text(encoding='utf-8')
start=text.index('pub fn complete_job(')
end=text.index('\npub fn fail_job(',start)
replacement=r'''pub fn complete_job(
    connection: &Connection,
    request: CompleteJobRequest,
) -> Result<ExecutionReceiptSummary> {
    reconcile_authority(connection)?;
    let worker_id = validate_uuid(&request.worker_id, "worker ID")?;
    let job_id = validate_uuid(&request.job_id, "job ID")?;
    let lease_token = bounded_text(&request.lease_token, 32, 128, "lease token")?;
    let requested_result_code = validate_symbol(&request.result_code, 120, "result code")?;
    let private_result_text = json_text(&request.private_result)?;
    ensure!((2..=MAX_PRIVATE_RESULT_BYTES).contains(&private_result_text.len()), "private job result exceeds the HomeServer limit");
    let private_provenance_text = json_text(&request.private_provenance)?;
    ensure!(private_provenance_text.len() <= MAX_PRIVATE_RESULT_BYTES, "private provenance exceeds the HomeServer limit");
    let transaction = connection.unchecked_transaction()?;
    let job = validate_worker_lease_tx(&transaction, &worker_id, &job_id, &lease_token, &["running"])?;
    ensure!(authority_is_current_tx(&transaction, &job)?, "job authority changed");
    let initial_safe_result = project_safe_result(&job, &request.private_result)?;
    let private_result_hash = hash_text(&private_result_text);
    let egress = wrapper_privacy::evaluate_egress_tx(
        &transaction,
        wrapper_privacy::EgressContext {
  job_id: &job.job_id,
  wrapper_id: &job.wrapper_id,
  connection_id: &job.connection_id,
  grant_id: &job.grant_id,
  grant_revision: job.grant_revision,
  connection_authority_revision: job.connection_authority_revision,
  capability_key: &job.capability_key,
        },
        &request.private_result,
        &initial_safe_result,
        &private_result_hash,
        request.source_count,
    )?;
    let provenance_summary = safe_provenance_summary(request.source_count, &request.source_types, request.evidence_hash.as_deref())?;
    let provenance_summary_text = json_text(&provenance_summary)?;
    let provenance_summary_hash = hash_text(&provenance_summary_text);
    let safe_result_text = egress.safe_result.as_ref().map(json_text).transpose()?;
    if let Some(text) = safe_result_text.as_deref() {
        ensure!(text.len() <= job.max_result_bytes as usize, "safe job result exceeds the captured grant limit");
    }
    enforce_completion_usage_tx(&transaction, &job.grant_id, safe_result_text.as_ref().map_or(0, String::len) as u64, request.actual_token_count.unwrap_or(0))?;
    let now = now_utc();
    transaction.execute(
        "INSERT INTO wrapper_job_private_results (job_id,private_result_json,private_provenance_json,private_result_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5)",
        params![job_id, private_result_text, private_provenance_text, private_result_hash, now],
    )?;
    let safe_result_hash = if let (Some(value), Some(text)) = (egress.safe_result.as_ref(), safe_result_text.as_ref()) {
        let hash = egress.output_hash.clone().unwrap_or(hash_json(value)?);
        transaction.execute(
  "INSERT INTO wrapper_job_safe_results (job_id,result_policy,safe_result_json,safe_result_hash,provenance_summary_json,provenance_summary_hash,filter_version,result_bytes,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
  params![job_id, job.result_policy, text, hash, provenance_summary_text, provenance_summary_hash, egress.filter_version, text.len() as i64, now],
        )?;
        Some(hash)
    } else { None };
    let (terminal_state, outcome, result_code) = match egress.state.as_str() {
        "denied" => ("failed", "error", egress.detail_code.clone()),
        "pending_review" => ("completed", "warning", "privacy_review_pending".to_owned()),
        _ => ("completed", "success", requested_result_code),
    };
    transaction.execute(
        "UPDATE wrapper_jobs SET state=?1,completed_at_utc=?2,lease_owner_id=NULL,lease_token_hash=NULL,lease_expires_at_utc=NULL,failure_code=CASE WHEN ?1='failed' THEN ?3 ELSE NULL END,updated_at_utc=?2 WHERE job_id=?4 AND state='running'",
        params![terminal_state, now, result_code, job_id],
    )?;
    let completed = job_record_by_id_tx(&transaction, &job_id)?;
    record_job_event(
        &transaction,
        &completed,
        JobEventEvidence {
  event_type: "wrapper.job.privacy_completed",
  previous_state: Some("running"),
  current_state: terminal_state,
  outcome,
  detail_code: &result_code,
  actor_type: "worker",
  actor_id: &worker_id,
  visibility: "wrapper",
  metadata: json!({
      "egress_decision_id": egress.decision_id,
      "egress_state": egress.state,
      "safe_result_hash": safe_result_hash,
      "provenance_summary_hash": provenance_summary_hash,
      "filter_version": egress.filter_version,
      "approval_required": egress.approval_required,
      "private_result_exposed": false,
      "source_identifiers_included": false,
      "private_source_content_included": false
  }),
        },
    )?;
    create_terminal_receipt_tx(
        &transaction,
        &completed,
        terminal_state,
        &result_code,
        safe_result_hash.as_deref(),
        Some(&provenance_summary_hash),
        Some(&worker_id),
    )?;
    transaction.execute("UPDATE wrapper_job_workers SET last_seen_at_utc=?1,updated_at_utc=?1 WHERE worker_id=?2", params![now, worker_id])?;
    transaction.commit()?;
    read_receipt(connection, &job_id)?.context("job completion receipt was not created")
}
'''
p.write_text(text[:start]+replacement+text[end:],encoding='utf-8')

patch('crates/homeserver-service/src/app/wrapper_jobs_reconcile.rs',
    '    Ok(base_current\n        && wrapper_agents::agent_job_authority_is_current_tx(transaction, &job.job_id)?)',
    '    Ok(base_current\n        && wrapper_agents::agent_job_authority_is_current_tx(transaction, &job.job_id)?\n        && wrapper_privacy::job_privacy_authority_is_current_tx(transaction, &job.job_id, &job.capability_key)?)')

p=Path('crates/homeserver-service/src/app/wrapper_jobs_delivery.rs')
text=p.read_text(encoding='utf-8')
old='''    let transaction = connection.unchecked_transaction()?;
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
'''
new='''    let transaction = connection.unchecked_transaction()?;
    let mut ready = Vec::new();
    for delivery in deliveries {
        if !wrapper_privacy::delivery_egress_is_current_tx(&transaction, &delivery.job_id)? {
            continue;
        }
        let delay_seconds = (15_i64 * 2_i64.pow(delivery.attempt_count.min(8))).min(3_600);
        let next_attempt = (Utc::now() + Duration::seconds(delay_seconds))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        transaction.execute(
            "UPDATE wrapper_job_deliveries SET state='in_flight',attempt_count=attempt_count+1,last_attempt_at_utc=?1,next_attempt_at_utc=?2,updated_at_utc=?1 WHERE delivery_id=?3 AND connection_id=?4 AND state IN ('pending','in_flight')",
            params![now, next_attempt, delivery.delivery_id, connection_id],
        )?;
        ready.push(delivery);
    }
    transaction.commit()?;
    ready
        .into_iter()
'''
if old not in text: raise SystemExit('delivery patch marker not found')
p.write_text(text.replace(old,new,1),encoding='utf-8')

patch('crates/homeserver-service/src/app/wrapper_jobs_read.rs',
    '    let safe_result = read_safe_result(connection, &job.job_id)?;',
    '    let safe_result = if wrapper_privacy::safe_result_is_visible(connection, &job.job_id)? {\n        read_safe_result(connection, &job.job_id)?\n    } else {\n        None\n    };')
