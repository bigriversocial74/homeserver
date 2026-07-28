PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS review_intelligence_settings (
    settings_id INTEGER PRIMARY KEY CHECK (settings_id=1),
    provider TEXT NOT NULL DEFAULT 'disabled' CHECK (provider IN ('disabled','ollama','openai')),
    model_name TEXT,
    remote_context_allowed INTEGER NOT NULL DEFAULT 0 CHECK (remote_context_allowed IN (0,1)),
    automatic_processing INTEGER NOT NULL DEFAULT 0 CHECK (automatic_processing IN (0,1)),
    minimum_cluster_size INTEGER NOT NULL DEFAULT 3 CHECK (minimum_cluster_size BETWEEN 2 AND 100),
    negative_sentiment_threshold REAL NOT NULL DEFAULT -0.25 CHECK (negative_sentiment_threshold BETWEEN -1.0 AND 1.0),
    campaign_drafting_enabled INTEGER NOT NULL DEFAULT 1 CHECK (campaign_drafting_enabled IN (0,1)),
    campaign_execution_enabled INTEGER NOT NULL DEFAULT 0 CHECK (campaign_execution_enabled IN (0,1)),
    updated_by TEXT NOT NULL DEFAULT 'local_control_center',
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT OR IGNORE INTO review_intelligence_settings (settings_id) VALUES (1);

CREATE TABLE IF NOT EXISTS review_intelligence_runs (
    run_id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    requested_provider TEXT NOT NULL CHECK (requested_provider IN ('deterministic','ollama','openai')),
    model_name TEXT,
    state TEXT NOT NULL CHECK (state IN ('running','completed','completed_with_errors','failed')),
    records_considered INTEGER NOT NULL DEFAULT 0,
    observations_created INTEGER NOT NULL DEFAULT 0,
    clusters_created INTEGER NOT NULL DEFAULT 0,
    recommendations_created INTEGER NOT NULL DEFAULT 0,
    remote_context_sent INTEGER NOT NULL DEFAULT 0 CHECK (remote_context_sent IN (0,1)),
    input_hash TEXT NOT NULL,
    output_hash TEXT,
    failure_code TEXT,
    started_at_utc TEXT NOT NULL,
    completed_at_utc TEXT,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_review_runs_connection_started
ON review_intelligence_runs(connection_id,started_at_utc DESC);

CREATE TABLE IF NOT EXISTS review_observations (
    observation_id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    dataset_key TEXT NOT NULL,
    source_object_type TEXT NOT NULL,
    source_object_id TEXT NOT NULL,
    source_revision TEXT NOT NULL,
    citation TEXT NOT NULL,
    text_hash TEXT NOT NULL,
    rating REAL,
    sentiment_score REAL NOT NULL CHECK (sentiment_score BETWEEN -1.0 AND 1.0),
    sentiment_label TEXT NOT NULL CHECK (sentiment_label IN ('negative','mixed','neutral','positive')),
    emotional_intensity REAL NOT NULL DEFAULT 0 CHECK (emotional_intensity BETWEEN 0 AND 1.0),
    primary_category TEXT NOT NULL,
    categories_json TEXT NOT NULL,
    entities_json TEXT NOT NULL,
    commitments_json TEXT NOT NULL,
    text_preview TEXT NOT NULL,
    trust_state TEXT NOT NULL DEFAULT 'untrusted_provider_evidence' CHECK (trust_state='untrusted_provider_evidence'),
    observed_at_utc TEXT,
    created_at_utc TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    UNIQUE (connection_id,entity_id,source_revision),
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT,
    FOREIGN KEY (entity_id) REFERENCES operational_entities(entity_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_review_observations_category
ON review_observations(connection_id,primary_category,sentiment_score,updated_at_utc DESC);

CREATE TABLE IF NOT EXISTS review_clusters (
    cluster_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    cluster_key TEXT NOT NULL,
    label TEXT NOT NULL,
    summary TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('deterministic','model_refined')),
    observation_count INTEGER NOT NULL,
    average_sentiment REAL NOT NULL,
    average_rating REAL,
    trend_direction TEXT NOT NULL DEFAULT 'stable' CHECK (trend_direction IN ('improving','stable','declining','new')),
    confidence REAL NOT NULL CHECK (confidence BETWEEN 0 AND 1.0),
    likely_causes_json TEXT NOT NULL,
    suggested_fixes_json TEXT NOT NULL,
    evidence_json TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','acknowledged','resolved','dismissed')),
    created_at_utc TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    UNIQUE (run_id,cluster_key),
    FOREIGN KEY (run_id) REFERENCES review_intelligence_runs(run_id) ON DELETE CASCADE,
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS review_cluster_memberships (
    cluster_id TEXT NOT NULL,
    observation_id TEXT NOT NULL,
    relevance REAL NOT NULL DEFAULT 1.0 CHECK (relevance BETWEEN 0 AND 1.0),
    PRIMARY KEY (cluster_id,observation_id),
    FOREIGN KEY (cluster_id) REFERENCES review_clusters(cluster_id) ON DELETE CASCADE,
    FOREIGN KEY (observation_id) REFERENCES review_observations(observation_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS review_recommendations (
    recommendation_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    cluster_id TEXT,
    connection_id TEXT NOT NULL,
    title TEXT NOT NULL,
    rationale TEXT NOT NULL,
    recommendation_type TEXT NOT NULL CHECK (recommendation_type IN ('operational_fix','staffing','inventory','product','service_recovery','campaign','follow_up','training','process')),
    severity TEXT NOT NULL CHECK (severity IN ('low','medium','high','critical')),
    confidence REAL NOT NULL CHECK (confidence BETWEEN 0 AND 1.0),
    suggested_actions_json TEXT NOT NULL,
    campaign_draft_json TEXT,
    evidence_json TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'proposed' CHECK (state IN ('proposed','accepted','dismissed','implemented','measuring','successful','unsuccessful')),
    created_at_utc TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    FOREIGN KEY (run_id) REFERENCES review_intelligence_runs(run_id) ON DELETE CASCADE,
    FOREIGN KEY (cluster_id) REFERENCES review_clusters(cluster_id) ON DELETE SET NULL,
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_review_recommendations_state
ON review_recommendations(connection_id,state,severity,updated_at_utc DESC);

CREATE TABLE IF NOT EXISTS review_recommendation_outcomes (
    outcome_id TEXT PRIMARY KEY,
    recommendation_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('accepted','dismissed','implemented','measuring','successful','unsuccessful')),
    note TEXT,
    evidence_json TEXT NOT NULL,
    recorded_by TEXT NOT NULL,
    recorded_at_utc TEXT NOT NULL,
    FOREIGN KEY (recommendation_id) REFERENCES review_recommendations(recommendation_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS review_model_receipts (
    receipt_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    provider TEXT NOT NULL CHECK (provider IN ('ollama','openai')),
    model_name TEXT NOT NULL,
    remote_context_sent INTEGER NOT NULL CHECK (remote_context_sent IN (0,1)),
    context_record_count INTEGER NOT NULL,
    input_hash TEXT NOT NULL,
    output_hash TEXT NOT NULL,
    response_identifier TEXT,
    duration_ms INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('completed','failed')),
    failure_code TEXT,
    created_at_utc TEXT NOT NULL,
    FOREIGN KEY (run_id) REFERENCES review_intelligence_runs(run_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS provider_operational_sync_receipts (
    receipt_id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    dataset_key TEXT NOT NULL,
    import_mode TEXT NOT NULL CHECK (import_mode IN ('snapshot','incremental','event')),
    provider_payload_hash TEXT NOT NULL,
    provider_source_revision TEXT,
    cursor_before TEXT,
    cursor_after TEXT,
    local_import_run_id TEXT,
    state TEXT NOT NULL CHECK (state IN ('completed','empty','failed','rejected')),
    failure_code TEXT,
    records_received INTEGER NOT NULL DEFAULT 0,
    events_received INTEGER NOT NULL DEFAULT 0,
    created_at_utc TEXT NOT NULL,
    completed_at_utc TEXT NOT NULL,
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT,
    FOREIGN KEY (local_import_run_id) REFERENCES operational_import_runs(import_run_id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS provider_campaign_action_receipts (
    receipt_id TEXT PRIMARY KEY,
    plan_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    recommendation_id TEXT,
    action_type TEXT NOT NULL,
    campaign_type TEXT NOT NULL,
    provider_receipt_id TEXT,
    provider_disposition TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    policy_hash TEXT,
    provider_response_json TEXT NOT NULL,
    created_at_utc TEXT NOT NULL,
    FOREIGN KEY (plan_id) REFERENCES agent_plans(plan_id) ON DELETE RESTRICT,
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT,
    FOREIGN KEY (recommendation_id) REFERENCES review_recommendations(recommendation_id) ON DELETE SET NULL
);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0013_review_intelligence_campaign_actions');
