-- Separate HomeServer software licensing/update authority from provider connections.
-- VP3 is the target authority. The legacy Microgifter gate remains active only
-- until a verified VP3 device registration and entitlement lease complete cutover.

CREATE TABLE IF NOT EXISTS homeserver_software_authority (
  singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
  current_authority TEXT NOT NULL DEFAULT 'microgifter_legacy' CHECK (
    current_authority IN ('microgifter_legacy','vp3')
  ),
  target_authority TEXT NOT NULL DEFAULT 'vp3' CHECK (target_authority = 'vp3'),
  cutover_state TEXT NOT NULL DEFAULT 'awaiting_vp3_activation' CHECK (
    cutover_state IN ('awaiting_vp3_activation','active','error')
  ),
  vp3_device_id TEXT,
  vp3_license_id TEXT,
  vp3_lease_id TEXT,
  vp3_lease_expires_at_utc TEXT,
  update_eligible INTEGER NOT NULL DEFAULT 0 CHECK (update_eligible IN (0,1)),
  allowed_update_channels_json TEXT NOT NULL DEFAULT '[]',
  last_vp3_heartbeat_at_utc TEXT,
  last_error_code TEXT,
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT OR IGNORE INTO homeserver_software_authority (
  singleton_id,current_authority,target_authority,cutover_state
) VALUES (1,'microgifter_legacy','vp3','awaiting_vp3_activation');

CREATE TABLE IF NOT EXISTS software_authority_receipts (
  receipt_id TEXT PRIMARY KEY,
  authority_key TEXT NOT NULL CHECK (authority_key IN ('microgifter_legacy','vp3')),
  event_type TEXT NOT NULL,
  update_id TEXT,
  version TEXT,
  disposition TEXT,
  failure_code TEXT,
  submission_state TEXT NOT NULL CHECK (
    submission_state IN ('legacy_forwarded','pending_vp3_submission','submitted','failed')
  ),
  created_at_utc TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_software_authority_receipts_created
  ON software_authority_receipts (created_at_utc DESC,receipt_id DESC);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0015_vp3_software_authority');
