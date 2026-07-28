#!/usr/bin/env python3
from pathlib import Path

review_path = Path("crates/homeserver-service/src/review_intelligence.rs")
review = review_path.read_text(encoding="utf-8")
old_loop = '''        for dataset_key in &dataset_keys {
            for _ in 0..AUTOMATIC_MAX_PAGES_PER_DATASET {
                let request = ProviderDatasetSyncRequest {
                    connection_id: connection_id.clone(),
                    dataset_key: dataset_key.clone(),
                    import_mode: Some("incremental".to_owned()),
                    limit: Some(AUTOMATIC_SYNC_PAGE_LIMIT),
                };
                match sync_provider_dataset_for_state(state.clone(), request).await {
                    Ok(result) => {
                        summary.datasets_synchronized += 1;
                        summary.records_received += result.records_received;
                        summary.events_received += result.events_received;
                        if result.records_received + result.events_received
                            < u64::from(AUTOMATIC_SYNC_PAGE_LIMIT)
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        summary.failed_operations += 1;
                        tracing::warn!(
                            ?error,
                            %connection_id,
                            %dataset_key,
                            "automatic Review Intelligence dataset sync failed"
                        );
                        break;
                    }
                }
            }
        }
'''
new_loop = '''        for dataset_key in &dataset_keys {
            let mut synchronized = false;
            for _ in 0..AUTOMATIC_MAX_PAGES_PER_DATASET {
                let request = ProviderDatasetSyncRequest {
                    connection_id: connection_id.clone(),
                    dataset_key: dataset_key.clone(),
                    import_mode: Some("incremental".to_owned()),
                    limit: Some(AUTOMATIC_SYNC_PAGE_LIMIT),
                };
                match sync_provider_dataset_for_state(state.clone(), request).await {
                    Ok(result) => {
                        synchronized = true;
                        summary.records_received += result.records_received;
                        summary.events_received += result.events_received;
                        if result.records_received + result.events_received
                            < u64::from(AUTOMATIC_SYNC_PAGE_LIMIT)
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        summary.failed_operations += 1;
                        tracing::warn!(
                            ?error,
                            %connection_id,
                            %dataset_key,
                            "automatic Review Intelligence dataset sync failed"
                        );
                        break;
                    }
                }
            }
            if synchronized {
                summary.datasets_synchronized += 1;
            }
        }
'''
if old_loop in review:
    review = review.replace(old_loop, new_loop, 1)
elif new_loop not in review:
    raise SystemExit("automatic dataset metric loop anchor was not found")
review_path.write_text(review, encoding="utf-8", newline="\n")

ui_path = Path("src/review-intelligence.js")
ui = ui_path.read_text(encoding="utf-8")
old_label = 'Process new evidence automatically after synchronization'
new_label = 'Automatically sync and analyze enabled review and message datasets every 15 minutes'
if old_label in ui:
    ui = ui.replace(old_label, new_label, 1)
elif new_label not in ui:
    raise SystemExit("automatic processing UI label anchor was not found")
ui_path.write_text(ui, encoding="utf-8", newline="\n")

validator_path = Path("scripts/validate-review-intelligence.py")
validator = validator_path.read_text(encoding="utf-8")
marker = '    "remote_context_allowed",\n'
replacement = marker + '    "every 15 minutes",\n'
if '"every 15 minutes"' not in validator:
    if validator.count(marker) != 1:
        raise SystemExit("automatic processing UI validator anchor was not found")
    validator = validator.replace(marker, replacement, 1)
validator_path.write_text(validator, encoding="utf-8", newline="\n")

print("Automatic Review Intelligence cadence and dataset metrics clarified.")
