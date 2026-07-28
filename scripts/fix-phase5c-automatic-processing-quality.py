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

app_path = Path("crates/homeserver-service/src/app.rs")
app = app_path.read_text(encoding="utf-8")
scheduler_anchor = '''async fn run_update_scheduler(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
'''
scheduler = r'''async fn run_review_intelligence_scheduler(
    state: Arc<AppState>,
    mut shutdown: watch::Receiver<bool>,
) {
    let start = tokio::time::Instant::now() + Duration::from_secs(2 * 60);
    let mut interval = tokio::time::interval_at(start, Duration::from_secs(15 * 60));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                match review_intelligence::run_automatic_processing_cycle(state.clone()).await {
                    Ok(summary) if summary.enabled && summary.failed_operations > 0 => {
                        warn!(
                            failures = summary.failed_operations,
                            connections = summary.connections_considered,
                            datasets = summary.datasets_synchronized,
                            "automatic Review Intelligence cycle completed with errors"
                        );
                    }
                    Ok(summary) if summary.enabled => {
                        info!(
                            connections = summary.connections_considered,
                            datasets = summary.datasets_synchronized,
                            records = summary.records_received,
                            events = summary.events_received,
                            analyses = summary.analyses_run,
                            "automatic Review Intelligence cycle completed"
                        );
                    }
                    Ok(_) => {}
                    Err(error) => warn!(?error, "automatic Review Intelligence cycle failed"),
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}

async fn run_update_scheduler(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
'''
if "async fn run_review_intelligence_scheduler(" not in app:
    if app.count(scheduler_anchor) != 1:
        raise SystemExit("review scheduler function anchor was not found")
    app = app.replace(scheduler_anchor, scheduler, 1)
app_path.write_text(app, encoding="utf-8", newline="\n")

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
old_markers = '''    "remote_context_allowed",
    "Run Deterministic Analysis",
'''
new_markers = '''    "remote_context_allowed",
    "every 15 minutes",
    "Run Deterministic Analysis",
'''
if '"every 15 minutes"' not in validator:
    if validator.count(old_markers) != 1:
        raise SystemExit("automatic processing UI validator anchor was not found")
    validator = validator.replace(old_markers, new_markers, 1)
validator_path.write_text(validator, encoding="utf-8", newline="\n")

print("Automatic Review Intelligence scheduler, cadence, and dataset metrics finalized.")
