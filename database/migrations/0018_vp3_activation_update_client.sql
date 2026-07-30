-- VP3 device activation, signed lease/release evidence, and update binding state.
-- Device credentials remain in the operating-system credential vault and never enter SQLite.

CREATE TABLE IF NOT EXISTS vp3_authority_client_state (
  singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
  account_id INTEGER,
  device_public_id TEXT,
  license_public_id TEXT,
  device_fingerprint TEXT,
  credential_key TEXT NOT NULL DEFAULT 'vp3-software-authority-device-credential',
  activation_state TEXT NOT NULL DEFAULT 'unconfigured' CHECK (
    activation_state IN ('unconfigured','activating','active','error')
  ),
  lease_public_id TEXT,
  lease_key_id TEXT,
  lease_document_hash TEXT,
  lease_signature_hash TEXT,
  lease_expires_at_utc TEXT,
  last_heartbeat_at_utc TEXT,
  last_manifest_checked_at_utc TEXT,
  last_error_code TEXT,
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT OR IGNORE INTO vp3_authority_client_state (singleton_id)
VALUES (1);

CREATE TABLE IF NOT EXISTS vp3_update_bindings (
  update_id TEXT PRIMARY KEY,
  release_public_id TEXT NOT NULL,
  version TEXT NOT NULL,
  channel TEXT NOT NULL CHECK (channel IN ('stable','security')),
  manifest_document TEXT NOT NULL,
  manifest_signature TEXT NOT NULL,
  signing_key_id TEXT NOT NULL,
  manifest_hash TEXT NOT NULL,
  installer_file_name TEXT NOT NULL,
  installer_sha256 TEXT NOT NULL,
  installer_size_bytes INTEGER NOT NULL CHECK (installer_size_bytes > 0),
  authenticode_thumbprint TEXT NOT NULL,
  checked_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_vp3_update_bindings_checked
  ON vp3_update_bindings (checked_at_utc DESC,update_id DESC);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0018_vp3_activation_update_client');
