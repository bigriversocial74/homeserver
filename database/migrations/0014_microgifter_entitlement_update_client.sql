-- Phase 6A: provider-neutral pairing, entitlement, capability, device and update-policy state.
-- This migration is additive. It does not replace the multi-cloud registry or signed updater.

CREATE TABLE IF NOT EXISTS provider_connection_profiles (
  connection_id TEXT PRIMARY KEY,
  provider_key TEXT NOT NULL CHECK (length(provider_key) BETWEEN 2 AND 40),
  contract_version TEXT NOT NULL DEFAULT 'v1',
  lifecycle_state TEXT NOT NULL CHECK (lifecycle_state IN (
    'unpaired','pairing_pending','active','offline','grace','suspended','revoked','replacing','error'
  )),
  owner_account_id TEXT,
  device_display_name TEXT NOT NULL CHECK (length(device_display_name) BETWEEN 1 AND 120),
  connector_version TEXT NOT NULL,
  capability_registry_version TEXT NOT NULL DEFAULT 'v1',
  entitlement_lease_id TEXT,
  entitlement_expires_at_utc TEXT,
  subscription_state TEXT CHECK (subscription_state IS NULL OR subscription_state IN (
    'active','grace','suspended','canceled','unknown'
  )),
  update_eligible INTEGER NOT NULL DEFAULT 0 CHECK (update_eligible IN (0,1)),
  last_heartbeat_at_utc TEXT,
  last_entitlement_refresh_at_utc TEXT,
  last_credential_rotation_at_utc TEXT,
  last_update_check_at_utc TEXT,
  last_update_result TEXT,
  replacement_state TEXT NOT NULL DEFAULT 'none' CHECK (replacement_state IN (
    'none','pending','activating','completed','failed'
  )),
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_provider_profiles_provider_state
  ON provider_connection_profiles (provider_key, lifecycle_state, updated_at_utc DESC);

CREATE TABLE IF NOT EXISTS provider_entitlement_signing_keys (
  provider_key TEXT NOT NULL,
  key_id TEXT NOT NULL,
  public_key_base64 TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','retired','revoked')),
  not_before_utc TEXT,
  not_after_utc TEXT,
  source TEXT NOT NULL CHECK (source IN ('compiled','mock_fixture')),
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (provider_key, key_id)
);

CREATE TABLE IF NOT EXISTS provider_entitlement_leases (
  lease_id TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL,
  provider_key TEXT NOT NULL,
  account_id TEXT NOT NULL,
  device_id TEXT NOT NULL,
  schema_version INTEGER NOT NULL,
  issued_at_utc TEXT NOT NULL,
  not_before_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  subscription_state TEXT NOT NULL CHECK (subscription_state IN (
    'active','grace','suspended','canceled','unknown'
  )),
  granted_capabilities_json TEXT NOT NULL DEFAULT '[]',
  denied_capabilities_json TEXT NOT NULL DEFAULT '[]',
  merchant_scope_json TEXT NOT NULL DEFAULT '[]',
  site_scope_json TEXT NOT NULL DEFAULT '[]',
  device_allowance_json TEXT NOT NULL DEFAULT '{}',
  update_eligibility INTEGER NOT NULL DEFAULT 0 CHECK (update_eligibility IN (0,1)),
  allowed_update_channels_json TEXT NOT NULL DEFAULT '[]',
  minimum_homeserver_version TEXT,
  signing_key_id TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  signature_base64 TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('accepted','expired','rejected','superseded')),
  accepted_at_utc TEXT,
  rejection_code TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_provider_entitlement_connection_state
  ON provider_entitlement_leases (connection_id, state, expires_at_utc DESC);

CREATE TABLE IF NOT EXISTS provider_connection_capabilities (
  connection_id TEXT NOT NULL,
  capability_id TEXT NOT NULL,
  grant_state TEXT NOT NULL CHECK (grant_state IN ('granted','denied','unavailable')),
  source TEXT NOT NULL CHECK (source IN ('client','account','device','lease','server')),
  expires_at_utc TEXT,
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (connection_id, capability_id, source),
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_provider_capabilities_connection_state
  ON provider_connection_capabilities (connection_id, grant_state, capability_id);

CREATE TABLE IF NOT EXISTS provider_connection_assignments (
  connection_id TEXT NOT NULL,
  assignment_type TEXT NOT NULL CHECK (assignment_type IN ('merchant','site')),
  assignment_id TEXT NOT NULL,
  parent_assignment_id TEXT,
  display_name TEXT,
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','suspended','revoked')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (connection_id, assignment_type, assignment_id),
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS provider_pairing_attempts (
  attempt_id TEXT PRIMARY KEY,
  provider_key TEXT NOT NULL,
  request_id TEXT NOT NULL UNIQUE,
  cloud_base_url TEXT NOT NULL,
  device_display_name TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('pending','completed','failed','expired')),
  connection_id TEXT,
  error_code TEXT,
  started_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS provider_connection_receipts (
  receipt_id TEXT PRIMARY KEY,
  provider_key TEXT NOT NULL,
  connection_id TEXT,
  device_id TEXT,
  event_type TEXT NOT NULL,
  request_id TEXT,
  previous_state TEXT,
  new_state TEXT,
  result_category TEXT NOT NULL CHECK (result_category IN ('success','warning','error','denied')),
  error_category TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_provider_receipts_connection_created
  ON provider_connection_receipts (connection_id, created_at_utc DESC, receipt_id DESC);

CREATE TABLE IF NOT EXISTS homeserver_update_preferences (
  singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
  selected_channel TEXT NOT NULL DEFAULT 'stable' CHECK (selected_channel IN ('stable','beta','preview')),
  install_mode TEXT NOT NULL DEFAULT 'install_now' CHECK (install_mode IN (
    'install_now','when_idle','tonight','maintenance_window','defer_until'
  )),
  maintenance_start_minute_utc INTEGER NOT NULL DEFAULT 120 CHECK (
    maintenance_start_minute_utc BETWEEN 0 AND 1439
  ),
  maintenance_duration_minutes INTEGER NOT NULL DEFAULT 180 CHECK (
    maintenance_duration_minutes BETWEEN 15 AND 720
  ),
  defer_until_utc TEXT,
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT OR IGNORE INTO homeserver_update_preferences (singleton_id) VALUES (1);

CREATE TABLE IF NOT EXISTS provider_update_authorizations (
  authorization_id TEXT PRIMARY KEY,
  connection_id TEXT,
  update_id TEXT NOT NULL,
  version TEXT NOT NULL,
  update_class TEXT NOT NULL CHECK (update_class IN (
    'bootstrap','security','maintenance','feature','preview','recovery'
  )),
  channel TEXT NOT NULL CHECK (channel IN ('stable','beta','preview')),
  decision TEXT NOT NULL CHECK (decision IN ('authorized','denied','not_required')),
  reason_code TEXT,
  issued_at_utc TEXT NOT NULL,
  expires_at_utc TEXT,
  receipt_submitted_at_utc TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE SET NULL,
  FOREIGN KEY (update_id) REFERENCES update_records(update_id) ON DELETE CASCADE,
  UNIQUE (connection_id, update_id)
);

CREATE INDEX IF NOT EXISTS idx_provider_update_authorizations_update
  ON provider_update_authorizations (update_id, decision, expires_at_utc DESC);

CREATE TABLE IF NOT EXISTS provider_device_replacements (
  replacement_id TEXT PRIMARY KEY,
  provider_key TEXT NOT NULL,
  old_connection_id TEXT,
  old_device_id TEXT,
  new_connection_id TEXT,
  new_device_id TEXT,
  state TEXT NOT NULL CHECK (state IN ('pending','paired','activated','completed','failed','canceled')),
  assignments_reviewed INTEGER NOT NULL DEFAULT 0 CHECK (assignments_reviewed IN (0,1)),
  grants_reviewed INTEGER NOT NULL DEFAULT 0 CHECK (grants_reviewed IN (0,1)),
  created_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  failure_code TEXT,
  FOREIGN KEY (old_connection_id) REFERENCES cloud_connections(connection_id) ON DELETE SET NULL,
  FOREIGN KEY (new_connection_id) REFERENCES cloud_connections(connection_id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS provider_device_identity_observations (
  observation_id TEXT PRIMARY KEY,
  provider_key TEXT NOT NULL,
  device_id TEXT NOT NULL,
  installation_id TEXT NOT NULL,
  machine_fingerprint_hash TEXT NOT NULL,
  connection_id TEXT,
  disposition TEXT NOT NULL CHECK (disposition IN (
    'trusted','duplicate','replacement_pending','stale_restore','rejected'
  )),
  observed_at_utc TEXT NOT NULL,
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_provider_device_observations_identity
  ON provider_device_identity_observations (provider_key, device_id, observed_at_utc DESC);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0014_microgifter_entitlement_update_client');
