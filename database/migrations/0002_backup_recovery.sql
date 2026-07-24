INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0002_backup_recovery');

CREATE TABLE IF NOT EXISTS backup_records (
  backup_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL
    CHECK (kind IN ('automatic', 'manual', 'recovery', 'pre_update')),
  encryption TEXT NOT NULL
    CHECK (encryption IN ('device_key_aes256gcm', 'passphrase_argon2id_aes256gcm')),
  state TEXT NOT NULL DEFAULT 'creating'
    CHECK (state IN ('creating', 'ready', 'verified', 'restore_staged', 'restored', 'failed')),
  file_name TEXT NOT NULL,
  storage_path TEXT NOT NULL UNIQUE,
  size_bytes INTEGER NOT NULL DEFAULT 0 CHECK (size_bytes >= 0),
  archive_sha256 TEXT,
  database_sha256 TEXT,
  note TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  verified_at_utc TEXT,
  restored_at_utc TEXT,
  failure_code TEXT,
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_backup_records_created
  ON backup_records (created_at_utc DESC, backup_id DESC);

CREATE INDEX IF NOT EXISTS idx_backup_records_retention
  ON backup_records (kind, state, created_at_utc DESC);

CREATE TABLE IF NOT EXISTS restore_requests (
  restore_id TEXT PRIMARY KEY,
  backup_id TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'staging'
    CHECK (state IN ('staging', 'staged', 'applying', 'applied', 'rolled_back', 'failed', 'cancelled')),
  pending_database_path TEXT,
  rollback_database_path TEXT,
  confirmation TEXT NOT NULL,
  requested_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  applied_at_utc TEXT,
  failure_code TEXT,
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  FOREIGN KEY (backup_id) REFERENCES backup_records (backup_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_restore_requests_state
  ON restore_requests (state, requested_at_utc DESC);

CREATE TABLE IF NOT EXISTS update_records (
  update_id TEXT PRIMARY KEY,
  version TEXT NOT NULL,
  channel TEXT NOT NULL DEFAULT 'stable',
  state TEXT NOT NULL DEFAULT 'discovered'
    CHECK (state IN ('discovered', 'verified', 'downloaded', 'preparing', 'ready', 'applied', 'failed', 'cancelled')),
  manifest_url TEXT,
  installer_path TEXT,
  installer_sha256 TEXT,
  manifest_signature TEXT,
  pre_update_backup_id TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  failure_code TEXT,
  FOREIGN KEY (pre_update_backup_id) REFERENCES backup_records (backup_id) ON DELETE RESTRICT
);

INSERT OR IGNORE INTO homeserver_settings (setting_key, setting_value)
VALUES
  ('backup_retention_count', '14'),
  ('backup_interval_hours', '24');
