PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS operational_provider_manifests (
    provider_key TEXT PRIMARY KEY,
    schema_version TEXT NOT NULL,
    manifest_json TEXT NOT NULL,
    manifest_hash TEXT NOT NULL,
    authority TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','disabled')),
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS operational_dataset_catalog (
    provider_key TEXT NOT NULL,
    dataset_key TEXT NOT NULL,
    label TEXT NOT NULL,
    description TEXT NOT NULL,
    authority TEXT NOT NULL,
    sensitivity TEXT NOT NULL CHECK (sensitivity IN ('public','business','restricted','sensitive')),
    sync_modes_json TEXT NOT NULL,
    default_retention_days INTEGER NOT NULL CHECK (default_retention_days BETWEEN 1 AND 3650),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    PRIMARY KEY (provider_key,dataset_key),
    FOREIGN KEY (provider_key) REFERENCES operational_provider_manifests(provider_key) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS operational_dataset_grants (
    grant_id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    tenant_id TEXT,
    site_id TEXT,
    dataset_key TEXT NOT NULL,
    import_direction TEXT NOT NULL DEFAULT 'provider_to_homeserver' CHECK (import_direction='provider_to_homeserver'),
    classification TEXT NOT NULL CHECK (classification IN ('public','business','restricted','sensitive')),
    retention_days INTEGER NOT NULL CHECK (retention_days BETWEEN 1 AND 3650),
    permitted_agent_uses_json TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'enabled' CHECK (state IN ('enabled','paused','revoked')),
    approved_by TEXT NOT NULL,
    approved_at_utc TEXT NOT NULL,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE (connection_id,dataset_key),
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT,
    FOREIGN KEY (provider_key,dataset_key) REFERENCES operational_dataset_catalog(provider_key,dataset_key) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS operational_import_runs (
    import_run_id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    tenant_id TEXT,
    site_id TEXT,
    dataset_key TEXT NOT NULL,
    import_mode TEXT NOT NULL CHECK (import_mode IN ('snapshot','incremental','event')),
    state TEXT NOT NULL CHECK (state IN ('running','completed','completed_with_errors','failed','quarantined')),
    cursor_before TEXT,
    cursor_after TEXT,
    source_revision TEXT,
    records_received INTEGER NOT NULL DEFAULT 0,
    records_imported INTEGER NOT NULL DEFAULT 0,
    records_rejected INTEGER NOT NULL DEFAULT 0,
    events_received INTEGER NOT NULL DEFAULT 0,
    failure_code TEXT,
    started_at_utc TEXT NOT NULL,
    completed_at_utc TEXT,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_operational_import_runs_connection_dataset
ON operational_import_runs(connection_id,dataset_key,started_at_utc DESC);

CREATE TABLE IF NOT EXISTS operational_import_cursors (
    connection_id TEXT NOT NULL,
    dataset_key TEXT NOT NULL,
    cursor_value TEXT,
    source_revision TEXT,
    last_successful_sync_utc TEXT,
    last_attempt_utc TEXT,
    next_scheduled_sync_utc TEXT,
    records_received INTEGER NOT NULL DEFAULT 0,
    records_rejected INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (connection_id,dataset_key),
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS operational_raw_records (
    raw_record_id TEXT PRIMARY KEY,
    import_run_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    tenant_id TEXT,
    site_id TEXT,
    dataset_key TEXT NOT NULL,
    source_object_type TEXT NOT NULL,
    source_object_id TEXT NOT NULL,
    source_revision TEXT NOT NULL,
    source_updated_at_utc TEXT,
    classification TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    trust_state TEXT NOT NULL DEFAULT 'untrusted_provider_evidence' CHECK (trust_state='untrusted_provider_evidence'),
    state TEXT NOT NULL DEFAULT 'accepted' CHECK (state IN ('accepted','quarantined','superseded')),
    received_at_utc TEXT NOT NULL,
    retention_until_utc TEXT NOT NULL,
    UNIQUE (connection_id,dataset_key,source_object_type,source_object_id,source_revision),
    FOREIGN KEY (import_run_id) REFERENCES operational_import_runs(import_run_id) ON DELETE CASCADE,
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_operational_raw_records_lookup
ON operational_raw_records(connection_id,dataset_key,source_object_type,source_object_id,received_at_utc DESC);

CREATE TABLE IF NOT EXISTS operational_entities (
    entity_id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    tenant_id TEXT,
    site_id TEXT,
    dataset_key TEXT NOT NULL,
    source_object_type TEXT NOT NULL,
    source_object_id TEXT NOT NULL,
    current_source_revision TEXT NOT NULL,
    current_payload_hash TEXT NOT NULL,
    current_payload_json TEXT NOT NULL,
    classification TEXT NOT NULL,
    source_updated_at_utc TEXT,
    received_at_utc TEXT NOT NULL,
    retention_until_utc TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','archived','deleted')),
    created_at_utc TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    UNIQUE (connection_id,dataset_key,source_object_type,source_object_id),
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_operational_entities_dataset
ON operational_entities(connection_id,dataset_key,updated_at_utc DESC);

CREATE TABLE IF NOT EXISTS operational_entity_versions (
    version_id TEXT PRIMARY KEY,
    entity_id TEXT NOT NULL,
    raw_record_id TEXT NOT NULL,
    source_revision TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    effective_at_utc TEXT,
    received_at_utc TEXT NOT NULL,
    UNIQUE (entity_id,source_revision),
    FOREIGN KEY (entity_id) REFERENCES operational_entities(entity_id) ON DELETE CASCADE,
    FOREIGN KEY (raw_record_id) REFERENCES operational_raw_records(raw_record_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS operational_events (
    event_id TEXT PRIMARY KEY,
    import_run_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    tenant_id TEXT,
    site_id TEXT,
    dataset_key TEXT NOT NULL,
    event_type TEXT NOT NULL,
    source_event_id TEXT NOT NULL,
    source_revision TEXT,
    occurred_at_utc TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    trust_state TEXT NOT NULL DEFAULT 'untrusted_provider_evidence' CHECK (trust_state='untrusted_provider_evidence'),
    received_at_utc TEXT NOT NULL,
    retention_until_utc TEXT NOT NULL,
    UNIQUE (connection_id,dataset_key,source_event_id),
    FOREIGN KEY (import_run_id) REFERENCES operational_import_runs(import_run_id) ON DELETE CASCADE,
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_operational_events_timeline
ON operational_events(connection_id,dataset_key,occurred_at_utc DESC);

CREATE TABLE IF NOT EXISTS operational_provenance (
    provenance_id TEXT PRIMARY KEY,
    entity_id TEXT,
    event_id TEXT,
    import_run_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    tenant_id TEXT,
    site_id TEXT,
    dataset_key TEXT NOT NULL,
    source_object_type TEXT,
    source_object_id TEXT,
    source_revision TEXT,
    evidence_hash TEXT NOT NULL,
    received_at_utc TEXT NOT NULL,
    CHECK ((entity_id IS NOT NULL) != (event_id IS NOT NULL)),
    FOREIGN KEY (entity_id) REFERENCES operational_entities(entity_id) ON DELETE CASCADE,
    FOREIGN KEY (event_id) REFERENCES operational_events(event_id) ON DELETE CASCADE,
    FOREIGN KEY (import_run_id) REFERENCES operational_import_runs(import_run_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS operational_retention_policies (
    connection_id TEXT NOT NULL,
    dataset_key TEXT NOT NULL,
    retention_days INTEGER NOT NULL CHECK (retention_days BETWEEN 1 AND 3650),
    disconnect_policy TEXT NOT NULL DEFAULT 'retain' CHECK (disconnect_policy IN ('retain','archive','erase')),
    updated_by TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    PRIMARY KEY (connection_id,dataset_key),
    FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS operational_import_errors (
    import_error_id TEXT PRIMARY KEY,
    import_run_id TEXT NOT NULL,
    record_index INTEGER,
    source_object_id TEXT,
    error_code TEXT NOT NULL,
    message TEXT NOT NULL,
    payload_hash TEXT,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    FOREIGN KEY (import_run_id) REFERENCES operational_import_runs(import_run_id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0012_operational_data_import');
