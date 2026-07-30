-- Local non-secret settings authority and VP3 synchronization state.
-- Provider credentials, private content, prompts, models, and files never enter this store.

CREATE TABLE IF NOT EXISTS federated_setting_catalog (
  setting_key TEXT PRIMARY KEY,
  label TEXT NOT NULL,
  description TEXT NOT NULL,
  category TEXT NOT NULL,
  authority TEXT NOT NULL CHECK (authority IN ('vp3','homeserver','shared')),
  value_type TEXT NOT NULL CHECK (value_type IN ('boolean','integer','string','enum')),
  default_value_json TEXT NOT NULL,
  allowed_values_json TEXT,
  visible_in_vp3 INTEGER NOT NULL DEFAULT 1 CHECK (visible_in_vp3 IN (0,1)),
  visible_in_homeserver INTEGER NOT NULL DEFAULT 1 CHECK (visible_in_homeserver IN (0,1)),
  sensitivity TEXT NOT NULL DEFAULT 'non_secret' CHECK (sensitivity='non_secret'),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS federated_setting_values (
  setting_key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL,
  value_hash TEXT NOT NULL,
  local_revision INTEGER NOT NULL DEFAULT 0 CHECK (local_revision >= 0),
  cloud_revision INTEGER NOT NULL DEFAULT 0 CHECK (cloud_revision >= 0),
  source_authority TEXT NOT NULL CHECK (source_authority IN ('default','vp3','homeserver')),
  dirty INTEGER NOT NULL DEFAULT 0 CHECK (dirty IN (0,1)),
  last_conflict_reason TEXT,
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (setting_key) REFERENCES federated_setting_catalog(setting_key) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS federated_settings_sync_state (
  singleton_id INTEGER PRIMARY KEY CHECK (singleton_id=1),
  max_cloud_revision INTEGER NOT NULL DEFAULT 0 CHECK (max_cloud_revision >= 0),
  snapshot_hash TEXT,
  last_synced_at_utc TEXT,
  last_error_code TEXT,
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT OR IGNORE INTO federated_settings_sync_state (singleton_id) VALUES (1);

CREATE TABLE IF NOT EXISTS federated_settings_sync_receipts (
  receipt_id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE,
  direction TEXT NOT NULL CHECK (direction IN ('local_update','device_sync')),
  base_revision INTEGER NOT NULL DEFAULT 0,
  applied_revision INTEGER NOT NULL DEFAULT 0,
  snapshot_hash TEXT,
  result TEXT NOT NULL CHECK (result IN ('applied','partial','conflict','failed')),
  conflict_count INTEGER NOT NULL DEFAULT 0,
  created_at_utc TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_federated_values_dirty
  ON federated_setting_values (dirty,setting_key);
CREATE INDEX IF NOT EXISTS idx_federated_receipts_created
  ON federated_settings_sync_receipts (created_at_utc DESC,receipt_id DESC);

INSERT INTO federated_setting_catalog
(setting_key,label,description,category,authority,value_type,default_value_json,allowed_values_json,visible_in_vp3,visible_in_homeserver,sensitivity,updated_at_utc)
VALUES
('appearance.theme','Appearance','Use the same light, dark, or system appearance across VP3 and HomeServer.','appearance','shared','enum','"system"','["system","light","dark"]',1,1,'non_secret',strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('regional.locale','Language and locale','Preferred interface locale for supported VP3 and HomeServer surfaces.','regional','shared','string','"en-US"',NULL,1,1,'non_secret',strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('regional.timezone','Time zone','IANA time zone used for schedules, receipts, and operational timestamps.','regional','shared','string','"UTC"',NULL,1,1,'non_secret',strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('updates.channel','Update channel','Permitted HomeServer software update channel.','updates','vp3','enum','"stable"','["stable","beta","security"]',1,1,'non_secret',strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('updates.auto_download','Automatic downloads','Download verified HomeServer updates automatically when permitted.','updates','shared','boolean','false',NULL,1,1,'non_secret',strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('updates.install_window','Local install window','Local hour range when an already verified update may be installed.','updates','homeserver','string','"02:00-05:00"',NULL,0,1,'non_secret',strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('notifications.email_enabled','Email notifications','Allow VP3 operational and billing notifications by email.','notifications','vp3','boolean','true',NULL,1,1,'non_secret',strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('notifications.desktop_enabled','Desktop notifications','Allow local HomeServer desktop notifications.','notifications','homeserver','boolean','true',NULL,0,1,'non_secret',strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('privacy.telemetry_level','Operational telemetry','Choose the non-content operational telemetry level shared with VP3.','privacy','shared','enum','"essential"','["off","essential","diagnostic"]',1,1,'non_secret',strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('commerce.default_currency','Default commerce currency','Default ISO currency for new HomeServer-created commerce applications.','commerce','shared','enum','"usd"','["usd","cad","eur","gbp","aud"]',1,1,'non_secret',strftime('%Y-%m-%dT%H:%M:%fZ','now')),
('commerce.receipt_email_enabled','Commerce receipt email','Enable receipt email by default for newly authorized commerce applications.','commerce','shared','boolean','true',NULL,1,1,'non_secret',strftime('%Y-%m-%dT%H:%M:%fZ','now'))
ON CONFLICT(setting_key) DO UPDATE SET
  label=excluded.label,
  description=excluded.description,
  category=excluded.category,
  authority=excluded.authority,
  value_type=excluded.value_type,
  default_value_json=excluded.default_value_json,
  allowed_values_json=excluded.allowed_values_json,
  visible_in_vp3=excluded.visible_in_vp3,
  visible_in_homeserver=excluded.visible_in_homeserver,
  sensitivity=excluded.sensitivity,
  updated_at_utc=excluded.updated_at_utc;

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0019_federated_settings');
