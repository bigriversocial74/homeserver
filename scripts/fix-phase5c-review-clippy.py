#!/usr/bin/env python3
from pathlib import Path

path = Path("crates/homeserver-service/src/review_intelligence.rs")
text = path.read_text(encoding="utf-8")

old_hash = "serde_json::to_value(&evidence.iter().map(|item| (&item.entity_id, &item.source_revision)).collect::<Vec<_>>())?"
new_hash = "serde_json::to_value(evidence.iter().map(|item| (&item.entity_id, &item.source_revision)).collect::<Vec<_>>())?"
if old_hash not in text and new_hash not in text:
    raise SystemExit("input hash anchor was not found")
text = text.replace(old_hash, new_hash, 1)

old_success = '''                    store_model_receipt(
                        &model_state,
                        &run_id,
                        &provider_copy,
                        &model_copy,
                        remote_context_sent,
                        context_count,
                        &input_hash,
                        &output_hash,
                        response_id.as_deref(),
                        started.elapsed().as_millis() as u64,
                        "completed",
                        None,
                    )
'''
new_success = '''                    store_model_receipt(
                        &model_state,
                        ModelReceiptRecord {
                            run_id: &run_id,
                            provider: &provider_copy,
                            model: &model_copy,
                            remote_context_sent,
                            context_record_count: context_count,
                            input_hash: &input_hash,
                            output_hash: &output_hash,
                            response_identifier: response_id.as_deref(),
                            duration_ms: started.elapsed().as_millis() as u64,
                            state_name: "completed",
                            failure_code: None,
                        },
                    )
'''
if old_success not in text and new_success not in text:
    raise SystemExit("completed model receipt call anchor was not found")
text = text.replace(old_success, new_success, 1)

old_failure = '''                store_model_receipt(
                    &state,
                    &analysis.run_id,
                    &provider,
                    model_name.as_deref().unwrap_or("unknown"),
                    remote_context_sent,
                    analysis.model_context.len(),
                    &analysis.input_hash,
                    &sha256_hex(failure.as_bytes()),
                    None,
                    started.elapsed().as_millis() as u64,
                    "failed",
                    Some(&failure),
                )?;
'''
new_failure = '''                let output_hash = sha256_hex(failure.as_bytes());
                store_model_receipt(
                    &state,
                    ModelReceiptRecord {
                        run_id: &analysis.run_id,
                        provider: &provider,
                        model: model_name.as_deref().unwrap_or("unknown"),
                        remote_context_sent,
                        context_record_count: analysis.model_context.len(),
                        input_hash: &analysis.input_hash,
                        output_hash: &output_hash,
                        response_identifier: None,
                        duration_ms: started.elapsed().as_millis() as u64,
                        state_name: "failed",
                        failure_code: Some(&failure),
                    },
                )?;
'''
if old_failure not in text and new_failure not in text:
    raise SystemExit("failed model receipt call anchor was not found")
text = text.replace(old_failure, new_failure, 1)

old_function = '''fn store_model_receipt(state: &AppState, run_id: &str, provider: &str, model: &str, remote: bool, count: usize, input_hash: &str, output_hash: &str, response_id: Option<&str>, duration_ms: u64, state_name: &str, failure_code: Option<&str>) -> Result<()> {
    state.connection()?.execute("INSERT INTO review_model_receipts (receipt_id,run_id,provider,model_name,remote_context_sent,context_record_count,input_hash,output_hash,response_identifier,duration_ms,state,failure_code,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)", params![Uuid::new_v4().to_string(), run_id, provider, model, i64::from(remote), count, input_hash, output_hash, response_id, duration_ms, state_name, failure_code, now_string()])?;
    Ok(())
}
'''
new_function = '''struct ModelReceiptRecord<'a> {
    run_id: &'a str,
    provider: &'a str,
    model: &'a str,
    remote_context_sent: bool,
    context_record_count: usize,
    input_hash: &'a str,
    output_hash: &'a str,
    response_identifier: Option<&'a str>,
    duration_ms: u64,
    state_name: &'a str,
    failure_code: Option<&'a str>,
}

fn store_model_receipt(state: &AppState, receipt: ModelReceiptRecord<'_>) -> Result<()> {
    state.connection()?.execute(
        "INSERT INTO review_model_receipts (receipt_id,run_id,provider,model_name,remote_context_sent,context_record_count,input_hash,output_hash,response_identifier,duration_ms,state,failure_code,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![
            Uuid::new_v4().to_string(),
            receipt.run_id,
            receipt.provider,
            receipt.model,
            i64::from(receipt.remote_context_sent),
            receipt.context_record_count,
            receipt.input_hash,
            receipt.output_hash,
            receipt.response_identifier,
            receipt.duration_ms,
            receipt.state_name,
            receipt.failure_code,
            now_string()
        ],
    )?;
    Ok(())
}
'''
if old_function not in text and new_function not in text:
    raise SystemExit("model receipt helper anchor was not found")
text = text.replace(old_function, new_function, 1)

path.write_text(text, encoding="utf-8", newline="\n")
print("Review intelligence Clippy defects repaired.")
