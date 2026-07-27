CREATE TABLE IF NOT EXISTS cloud_connections (
  connection_id TEXT PRIMARY KEY,
  provider_key TEXT NOT NULL CHECK (length(provider_key) BETWEEN 2 AND 40),
  display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 120),
  cloud_base_url TEXT NOT NULL,
  tenant_id TEXT,
  site_id TEXT,
  device_id TEXT NOT NULL,
  public_key_base64 TEXT NOT NULL,
  credential_key TEXT NOT NULL UNIQUE,
  state TEXT NOT NULL CHECK (state IN ('pairing','connected','degraded','revoked','disconnected')),
  scopes_json TEXT NOT NULL DEFAULT '[]',
  is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0,1)),
  paired_at_utc TEXT NOT NULL,
  last_success_utc TEXT,
  last_error TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_cloud_connections_default
  ON cloud_connections (is_default)
  WHERE is_default = 1;

CREATE INDEX IF NOT EXISTS idx_cloud_connections_provider_state
  ON cloud_connections (provider_key, state, updated_at_utc DESC);

CREATE TABLE IF NOT EXISTS cloud_sync_queue (
  queue_id INTEGER PRIMARY KEY AUTOINCREMENT,
  connection_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  operation_type TEXT NOT NULL,
  payload_json TEXT NOT NULL DEFAULT '{}',
  state TEXT NOT NULL CHECK (state IN ('pending','processing','accepted','rejected','review')),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  available_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE,
  UNIQUE (connection_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_cloud_sync_queue_due
  ON cloud_sync_queue (connection_id, state, available_at_utc, queue_id);

CREATE TABLE IF NOT EXISTS cloud_sync_receipts (
  receipt_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  operation_type TEXT NOT NULL,
  disposition TEXT NOT NULL CHECK (disposition IN ('accepted','rejected','review')),
  reason_code TEXT,
  response_json TEXT NOT NULL DEFAULT '{}',
  received_at_utc TEXT NOT NULL,
  PRIMARY KEY (connection_id, receipt_id),
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE,
  UNIQUE (connection_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_cloud_sync_receipts_received
  ON cloud_sync_receipts (connection_id, received_at_utc DESC, receipt_id DESC);

CREATE TABLE IF NOT EXISTS cloud_connection_events (
  event_id TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL,
  event_type TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error')),
  detail_code TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_cloud_connection_events_recent
  ON cloud_connection_events (connection_id, created_at_utc DESC, event_id DESC);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0010_multi_cloud_connections');
