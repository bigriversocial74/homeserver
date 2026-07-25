CREATE TABLE IF NOT EXISTS cloud_connection (
  singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
  cloud_base_url TEXT NOT NULL,
  device_id TEXT NOT NULL,
  public_key_base64 TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('pairing','connected','degraded','revoked')),
  scopes_json TEXT NOT NULL DEFAULT '[]',
  paired_at_utc TEXT NOT NULL,
  last_success_utc TEXT,
  last_error TEXT,
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS sync_receipts (
  receipt_id TEXT PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  operation_type TEXT NOT NULL,
  disposition TEXT NOT NULL CHECK (disposition IN ('accepted','rejected','review')),
  reason_code TEXT,
  response_json TEXT NOT NULL DEFAULT '{}',
  received_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_sync_receipts_received
  ON sync_receipts (received_at_utc DESC, receipt_id DESC);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0004_cloud_pairing_sync');
