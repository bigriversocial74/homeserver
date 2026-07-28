#!/usr/bin/env python3
from pathlib import Path

review_path = Path("crates/homeserver-service/src/review_intelligence.rs")
review = review_path.read_text(encoding="utf-8")

constant_anchor = 'const MAX_MODEL_OUTPUT_CHARS: usize = 40_000;\n'
constant_replacement = constant_anchor + 'const AUTOMATIC_SYNC_PAGE_LIMIT: u32 = 250;\nconst AUTOMATIC_MAX_PAGES_PER_DATASET: usize = 4;\n'
if "AUTOMATIC_MAX_PAGES_PER_DATASET" not in review:
    if review.count(constant_anchor) != 1:
        raise SystemExit("automatic-processing constant anchor was not found")
    review = review.replace(constant_anchor, constant_replacement, 1)

result_anchor = '''#[derive(Debug, Clone, Deserialize)]
pub struct RecommendationOutcomeRequest {
'''
result_struct = '''#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AutomaticReviewCycleSummary {
    pub enabled: bool,
    pub connections_considered: u64,
    pub datasets_synchronized: u64,
    pub records_received: u64,
    pub events_received: u64,
    pub analyses_run: u64,
    pub failed_operations: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecommendationOutcomeRequest {
'''
if "struct AutomaticReviewCycleSummary" not in review:
    if review.count(result_anchor) != 1:
        raise SystemExit("automatic-processing result anchor was not found")
    review = review.replace(result_anchor, result_struct, 1)

function_anchor = '''async fn run_analysis_for_state(
'''
automatic_functions = r'''fn automatic_processing_targets(state: &AppState) -> Result<Vec<(String, Vec<String>)>> {
    let connection = state.connection()?;
    let mut statement = connection.prepare(
        "SELECT g.connection_id,g.dataset_key,g.permitted_agent_uses_json FROM operational_dataset_grants g JOIN cloud_connections c ON c.connection_id=g.connection_id WHERE g.state='enabled' AND c.provider_key='microgifter' AND c.state NOT IN ('revoked','disconnected') ORDER BY g.connection_id,g.dataset_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        let (connection_id, dataset_key, uses_json) = row?;
        if !REVIEW_DATASETS.contains(&dataset_key.as_str()) {
            continue;
        }
        let uses: Vec<String> = serde_json::from_str(&uses_json).unwrap_or_default();
        if !uses.iter().any(|value| value == "analyze") {
            continue;
        }
        grouped.entry(connection_id).or_default().push(dataset_key);
    }
    Ok(grouped.into_iter().collect())
}

fn automatic_analysis_due(
    state: &AppState,
    connection_id: &str,
    dataset_keys: &[String],
) -> Result<bool> {
    let connection = state.connection()?;
    let last_completed: Option<String> = connection
        .query_row(
            "SELECT MAX(completed_at_utc) FROM review_intelligence_runs WHERE connection_id=?1 AND state IN ('completed','completed_with_errors')",
            params![connection_id],
            |row| row.get(0),
        )?;
    for dataset_key in dataset_keys {
        let latest_received: Option<String> = connection.query_row(
            "SELECT MAX(received_at_utc) FROM operational_entities WHERE connection_id=?1 AND dataset_key=?2 AND state='active'",
            params![connection_id, dataset_key],
            |row| row.get(0),
        )?;
        if let Some(latest_received) = latest_received {
            if last_completed
                .as_deref()
                .is_none_or(|completed| latest_received.as_str() > completed)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(crate) async fn run_automatic_processing_cycle(
    state: Arc<AppState>,
) -> Result<AutomaticReviewCycleSummary> {
    let settings = {
        let connection = state.connection()?;
        read_settings(&connection)?
    };
    let mut summary = AutomaticReviewCycleSummary {
        enabled: settings.automatic_processing,
        connections_considered: 0,
        datasets_synchronized: 0,
        records_received: 0,
        events_received: 0,
        analyses_run: 0,
        failed_operations: 0,
    };
    if !settings.automatic_processing {
        return Ok(summary);
    }

    let target_state = state.clone();
    let targets = tokio::task::spawn_blocking(move || automatic_processing_targets(&target_state))
        .await
        .context("automatic review target task failed")??;
    for (connection_id, dataset_keys) in targets {
        summary.connections_considered += 1;
        for dataset_key in &dataset_keys {
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

        let due_state = state.clone();
        let due_connection_id = connection_id.clone();
        let due_dataset_keys = dataset_keys.clone();
        let analysis_due = tokio::task::spawn_blocking(move || {
            automatic_analysis_due(&due_state, &due_connection_id, &due_dataset_keys)
        })
        .await
        .context("automatic review due-state task failed")??;
        if !analysis_due {
            continue;
        }
        match run_analysis_for_state(
            state.clone(),
            RunReviewAnalysisRequest {
                connection_id: connection_id.clone(),
                dataset_keys: dataset_keys.clone(),
                use_llm: Some(settings.provider != "disabled"),
                maximum_records: Some(MAX_ANALYSIS_RECORDS as u32),
            },
        )
        .await
        {
            Ok(_) => summary.analyses_run += 1,
            Err(error) => {
                summary.failed_operations += 1;
                tracing::warn!(
                    ?error,
                    %connection_id,
                    "automatic Review Intelligence analysis failed"
                );
            }
        }
    }
    Ok(summary)
}

async fn run_analysis_for_state(
'''
if "run_automatic_processing_cycle" not in review:
    if review.count(function_anchor) != 1:
        raise SystemExit("automatic-processing function anchor was not found")
    review = review.replace(function_anchor, automatic_functions, 1)
review_path.write_text(review, encoding="utf-8", newline="\n")

app_path = Path("crates/homeserver-service/src/app.rs")
app = app_path.read_text(encoding="utf-8")
spawn_anchor = '''    let cloud_worker = tokio::spawn(cloud_registry::run(state.clone(), shutdown.clone()));
'''
spawn_replacement = spawn_anchor + '''    let review_intelligence_worker =
        tokio::spawn(run_review_intelligence_scheduler(state.clone(), shutdown.clone()));
'''
if "review_intelligence_worker" not in app:
    if app.count(spawn_anchor) != 1:
        raise SystemExit("review scheduler spawn anchor was not found")
    app = app.replace(spawn_anchor, spawn_replacement, 1)

abort_anchor = '''    cloud_worker.abort();
'''
abort_replacement = abort_anchor + '''    review_intelligence_worker.abort();
'''
if "review_intelligence_worker.abort()" not in app:
    if app.count(abort_anchor) != 1:
        raise SystemExit("review scheduler abort anchor was not found")
    app = app.replace(abort_anchor, abort_replacement, 1)

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
if "run_review_intelligence_scheduler" not in app:
    if app.count(scheduler_anchor) != 1:
        raise SystemExit("review scheduler function anchor was not found")
    app = app.replace(scheduler_anchor, scheduler, 1)
app_path.write_text(app, encoding="utf-8", newline="\n")

validator_path = Path("scripts/validate-review-intelligence.py")
validator = validator_path.read_text(encoding="utf-8")
service_marker_anchor = '''    "campaign_execution_enabled",
    "provider_campaign_action_receipts",
'''
service_marker_replacement = '''    "campaign_execution_enabled",
    "run_automatic_processing_cycle",
    "automatic_processing_targets",
    "automatic_analysis_due",
    "AUTOMATIC_MAX_PAGES_PER_DATASET",
    "provider_campaign_action_receipts",
'''
if '"run_automatic_processing_cycle"' not in validator:
    if validator.count(service_marker_anchor) != 1:
        raise SystemExit("automatic-processing validator service anchor was not found")
    validator = validator.replace(service_marker_anchor, service_marker_replacement, 1)

app_marker_anchor = '''require("crates/homeserver-service/src/app.rs", ".merge(review_intelligence::router(state.clone()))", "review intelligence router is not secured inside the local API")
'''
app_marker_replacement = app_marker_anchor + '''require("crates/homeserver-service/src/app.rs", "run_review_intelligence_scheduler", "automatic Review Intelligence scheduler is not registered")
require("crates/homeserver-service/src/app.rs", "Duration::from_secs(15 * 60)", "automatic Review Intelligence scheduler is not bounded to a 15-minute cadence")
'''
if "automatic Review Intelligence scheduler is not registered" not in validator:
    if validator.count(app_marker_anchor) != 1:
        raise SystemExit("automatic-processing validator app anchor was not found")
    validator = validator.replace(app_marker_anchor, app_marker_replacement, 1)
validator_path.write_text(validator, encoding="utf-8", newline="\n")

print("Bounded automatic Review Intelligence sync and analysis scheduler applied.")
