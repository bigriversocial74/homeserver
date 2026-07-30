-- Phase 16A: shared multi-wrapper identity and pairing foundation.
-- This migration is additive. Existing provider adapters remain authoritative for
-- remote pairing exchange and provider-specific synchronization during transition.
-- Secrets remain in the operating-system credential vault and never enter SQLite.

CREATE TABLE IF NOT EXISTS wrapper_identities (
  wrapper_id TEXT PRIMARY KEY,
  wrapper_key TEXT NOT NULL UNIQUE CHECK (
    length(wrapper_key) BETWEEN 2 AND 40
    AND wrapper_key = lower(wrapper_key)
  ),
  display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 120),
  wrapper_kind TEXT NOT NULL CHECK (wrapper_kind IN (
    'pod','application','commerce','media','service','other'
  )),
  protocol_version TEXT NOT NULL CHECK (length(protocol_version) BETWEEN 1 AND 40),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN (
    'active','suspended','revoked','retired'
  )),
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  revoked_at_utc TEXT
);

CREATE INDEX IF NOT EXISTS idx_wrapper_identities_state
  ON wrapper_identities (state, updated_at_utc DESC, wrapper_id);

CREATE TABLE IF NOT EXISTS wrapper_connections (
  connection_id TEXT PRIMARY KEY,
  wrapper_id TEXT NOT NULL,
  remote_connection_id TEXT,
  remote_origin TEXT NOT NULL,
  contract_version TEXT NOT NULL CHECK (length(contract_version) BETWEEN 1 AND 80),
  lifecycle_state TEXT NOT NULL CHECK (lifecycle_state IN (
    'unpaired','pairing_pending','active','offline','grace','suspended',
    'revoked','replacing','error','disconnected'
  )),
  credential_reference TEXT NOT NULL UNIQUE,
  grant_revision INTEGER NOT NULL DEFAULT 0 CHECK (grant_revision >= 0),
  legacy_provider_key TEXT,
  legacy_connection_id TEXT,
  paired_at_utc TEXT,
  last_seen_at_utc TEXT,
  revoked_at_utc TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE,
  UNIQUE (wrapper_id, remote_connection_id)
);

CREATE INDEX IF NOT EXISTS idx_wrapper_connections_wrapper_state
  ON wrapper_connections (wrapper_id, lifecycle_state, updated_at_utc DESC, connection_id);
CREATE INDEX IF NOT EXISTS idx_wrapper_connections_legacy
  ON wrapper_connections (legacy_provider_key, legacy_connection_id);

CREATE TABLE IF NOT EXISTS wrapper_devices (
  wrapper_device_id TEXT PRIMARY KEY,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL UNIQUE,
  device_public_id TEXT NOT NULL,
  installation_id TEXT NOT NULL,
  public_key_base64 TEXT NOT NULL,
  credential_reference TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN (
    'pairing_pending','active','offline','suspended','revoked','replacing','retired','error'
  )),
  last_seen_at_utc TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  revoked_at_utc TEXT,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  UNIQUE (wrapper_id, device_public_id)
);

CREATE INDEX IF NOT EXISTS idx_wrapper_devices_wrapper_state
  ON wrapper_devices (wrapper_id, state, updated_at_utc DESC, wrapper_device_id);

CREATE TABLE IF NOT EXISTS wrapper_pairing_attempts (
  attempt_id TEXT PRIMARY KEY,
  wrapper_id TEXT NOT NULL,
  request_id TEXT NOT NULL,
  request_hash TEXT NOT NULL,
  remote_origin TEXT NOT NULL,
  device_display_name TEXT NOT NULL CHECK (length(device_display_name) BETWEEN 1 AND 120),
  requested_capabilities_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(requested_capabilities_json)),
  state TEXT NOT NULL CHECK (state IN ('pending','completed','failed','expired','cancelled')),
  expires_at_utc TEXT NOT NULL,
  result_connection_id TEXT,
  error_code TEXT,
  created_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (result_connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE SET NULL,
  UNIQUE (wrapper_id, request_id)
);

CREATE INDEX IF NOT EXISTS idx_wrapper_pairing_attempts_due
  ON wrapper_pairing_attempts (state, expires_at_utc, created_at_utc DESC, attempt_id);

CREATE TABLE IF NOT EXISTS wrapper_credential_references (
  credential_reference TEXT PRIMARY KEY,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT,
  credential_kind TEXT NOT NULL CHECK (credential_kind IN (
    'connection_bundle','bearer_token','device_signing_key','job_lease','api_token','other'
  )),
  vault_service TEXT NOT NULL,
  vault_account TEXT NOT NULL,
  token_hint TEXT,
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN (
    'active','rotating','expired','revoked','missing','error'
  )),
  expires_at_utc TEXT,
  rotated_at_utc TEXT,
  revoked_at_utc TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  UNIQUE (connection_id, credential_kind)
);

CREATE INDEX IF NOT EXISTS idx_wrapper_credentials_wrapper_state
  ON wrapper_credential_references (wrapper_id, state, updated_at_utc DESC, credential_reference);

CREATE TABLE IF NOT EXISTS wrapper_events (
  event_id TEXT PRIMARY KEY,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT,
  event_type TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error','denied')),
  correlation_id TEXT,
  causation_id TEXT,
  visibility TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','security','operational')),
  detail_code TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_wrapper_events_wrapper_created
  ON wrapper_events (wrapper_id, created_at_utc DESC, event_id DESC);
CREATE INDEX IF NOT EXISTS idx_wrapper_events_connection_created
  ON wrapper_events (connection_id, created_at_utc DESC, event_id DESC);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0020_wrapper_identity_and_pairing');
