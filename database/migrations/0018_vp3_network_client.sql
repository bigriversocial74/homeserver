-- VP3 network client state and signed release authorization evidence.
ALTER TABLE homeserver_software_authority ADD COLUMN vp3_api_base_url TEXT;
ALTER TABLE homeserver_software_authority ADD COLUMN vp3_account_id INTEGER;
ALTER TABLE homeserver_software_authority ADD COLUMN vp3_device_fingerprint TEXT;
ALTER TABLE homeserver_software_authority ADD COLUMN vp3_credential_key TEXT;
ALTER TABLE homeserver_software_authority ADD COLUMN vp3_lease_document TEXT;
ALTER TABLE homeserver_software_authority ADD COLUMN vp3_lease_signature TEXT;
ALTER TABLE homeserver_software_authority ADD COLUMN vp3_lease_key_id TEXT;
ALTER TABLE homeserver_software_authority ADD COLUMN vp3_release_public_id TEXT;
ALTER TABLE homeserver_software_authority ADD COLUMN last_vp3_manifest_at_utc TEXT;

CREATE TABLE IF NOT EXISTS vp3_release_authorizations (
  update_id TEXT PRIMARY KEY,
  release_public_id TEXT NOT NULL,
  version TEXT NOT NULL,
  channel TEXT NOT NULL,
  signed_document TEXT NOT NULL,
  signature TEXT NOT NULL,
  signing_key_id TEXT NOT NULL,
  manifest_hash TEXT NOT NULL,
  installer_url TEXT NOT NULL,
  installer_file_name TEXT NOT NULL,
  installer_size_bytes INTEGER NOT NULL,
  installer_sha256 TEXT NOT NULL,
  authenticode_thumbprint TEXT NOT NULL,
  grant_credential_key TEXT NOT NULL,
  grant_expires_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS vp3_authority_outbox (
  receipt_id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE,
  update_id TEXT NOT NULL,
  release_public_id TEXT,
  version TEXT NOT NULL,
  disposition TEXT NOT NULL CHECK (disposition IN ('downloaded','staged','installed','rolled_back','failed')),
  failure_code TEXT,
  receipt_hash TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','submitted','failed')),
  attempt_count INTEGER NOT NULL DEFAULT 0,
  last_attempt_at_utc TEXT,
  submitted_at_utc TEXT,
  last_error_code TEXT,
  created_at_utc TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_vp3_authority_outbox_state
  ON vp3_authority_outbox (state,created_at_utc);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0018_vp3_network_client');
