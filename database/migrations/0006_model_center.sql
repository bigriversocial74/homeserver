CREATE TABLE IF NOT EXISTS model_operations (
    operation_id TEXT PRIMARY KEY,
    model_name TEXT NOT NULL,
    operation_type TEXT NOT NULL CHECK (operation_type IN ('pull','delete','unload','test')),
    state TEXT NOT NULL CHECK (state IN ('pending','running','succeeded','failed','interrupted')),
    status_message TEXT NOT NULL DEFAULT '',
    completed_bytes INTEGER NOT NULL DEFAULT 0 CHECK (completed_bytes >= 0),
    total_bytes INTEGER NOT NULL DEFAULT 0 CHECK (total_bytes >= 0),
    failure_code TEXT,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    completed_at_utc TEXT
);

CREATE INDEX IF NOT EXISTS idx_model_operations_updated
    ON model_operations(updated_at_utc DESC, operation_id DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_model_operations_active_pull
    ON model_operations(model_name)
    WHERE operation_type='pull' AND state IN ('pending','running');

INSERT OR IGNORE INTO homeserver_settings (setting_key, setting_value) VALUES
    ('model_default_chat', ''),
    ('model_default_embedding', ''),
    ('model_context_size', '4096'),
    ('model_test_timeout_seconds', '60'),
    ('model_max_download_gb', '20');

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0006_model_center');
