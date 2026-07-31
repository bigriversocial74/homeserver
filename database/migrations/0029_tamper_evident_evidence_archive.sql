-- Phase 21: tamper-evident evidence archive and independently verifiable export.
-- The archive copies only reviewed evidence tables. It never archives prompts,
-- model output, document content, credentials, private payloads, or arbitrary tables.
-- Source evidence remains immutable and is not deleted by this phase.

CREATE TABLE IF NOT EXISTS evidence_archive_policies (
  policy_id TEXT PRIMARY KEY,
  policy_revision INTEGER NOT NULL UNIQUE CHECK (policy_revision >= 1),
  enabled INTEGER NOT NULL CHECK (enabled IN (0,1)),
  interval_hours INTEGER NOT NULL CHECK (interval_hours BETWEEN 1 AND 720),
  max_records_per_archive INTEGER NOT NULL CHECK (max_records_per_archive BETWEEN 100 AND 50000),
  retention_count INTEGER NOT NULL CHECK (retention_count BETWEEN 1 AND 365),
  max_package_bytes INTEGER NOT NULL CHECK (max_package_bytes BETWEEN 1048576 AND 268435456),
  policy_hash TEXT NOT NULL UNIQUE CHECK (length(policy_hash)=64),
  created_by_user_id TEXT NOT NULL CHECK (length(created_by_user_id) BETWEEN 1 AND 160),
  reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
  created_at_utc TEXT NOT NULL
);

CREATE TRIGGER IF NOT EXISTS trg_evidence_archive_policies_no_update
BEFORE UPDATE ON evidence_archive_policies
BEGIN
  SELECT RAISE(ABORT,'evidence archive policy revisions are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_evidence_archive_policies_no_delete
BEFORE DELETE ON evidence_archive_policies
BEGIN
  SELECT RAISE(ABORT,'evidence archive policy revisions are immutable');
END;

CREATE TABLE IF NOT EXISTS evidence_archives (
  archive_id TEXT PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE CHECK (length(idempotency_key) BETWEEN 8 AND 160),
  policy_id TEXT NOT NULL,
  policy_revision INTEGER NOT NULL CHECK (policy_revision >= 1),
  state TEXT NOT NULL CHECK (state IN ('collecting','sealed','verified','failed')),
  previous_archive_id TEXT,
  previous_archive_hash TEXT NOT NULL CHECK (length(previous_archive_hash)=64),
  archive_sequence INTEGER NOT NULL UNIQUE CHECK (archive_sequence >= 1),
  record_count INTEGER NOT NULL DEFAULT 0 CHECK (record_count BETWEEN 0 AND 50000),
  table_count INTEGER NOT NULL DEFAULT 0 CHECK (table_count BETWEEN 0 AND 256),
  first_record_at_utc TEXT,
  last_record_at_utc TEXT,
  records_sha256 TEXT CHECK (records_sha256 IS NULL OR length(records_sha256)=64),
  chain_root_hash TEXT CHECK (chain_root_hash IS NULL OR length(chain_root_hash)=64),
  manifest_sha256 TEXT CHECK (manifest_sha256 IS NULL OR length(manifest_sha256)=64),
  package_sha256 TEXT CHECK (package_sha256 IS NULL OR length(package_sha256)=64),
  package_size_bytes INTEGER CHECK (package_size_bytes IS NULL OR package_size_bytes BETWEEN 1 AND 268435456),
  file_name TEXT NOT NULL CHECK (length(file_name) BETWEEN 1 AND 220),
  storage_path TEXT NOT NULL CHECK (length(storage_path) BETWEEN 1 AND 2048),
  encryption TEXT NOT NULL DEFAULT 'device_key_aes256gcm' CHECK (encryption='device_key_aes256gcm'),
  created_by_type TEXT NOT NULL CHECK (created_by_type IN ('local_user','system')),
  created_by_id TEXT NOT NULL CHECK (length(created_by_id) BETWEEN 1 AND 160),
  failure_code TEXT,
  created_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  verified_at_utc TEXT,
  FOREIGN KEY (policy_id) REFERENCES evidence_archive_policies(policy_id) ON DELETE RESTRICT,
  FOREIGN KEY (previous_archive_id) REFERENCES evidence_archives(archive_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_evidence_archives_recent
  ON evidence_archives (archive_sequence DESC, archive_id DESC);
CREATE INDEX IF NOT EXISTS idx_evidence_archives_state
  ON evidence_archives (state, created_at_utc DESC, archive_id DESC);

CREATE TRIGGER IF NOT EXISTS trg_evidence_archives_terminal_immutable
BEFORE UPDATE ON evidence_archives
WHEN OLD.state IN ('verified','failed')
BEGIN
  SELECT RAISE(ABORT,'terminal evidence archives are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_evidence_archives_no_delete
BEFORE DELETE ON evidence_archives
BEGIN
  SELECT RAISE(ABORT,'evidence archives are immutable');
END;

CREATE TABLE IF NOT EXISTS evidence_archive_members (
  member_id TEXT PRIMARY KEY,
  archive_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK (ordinal >= 1),
  source_table TEXT NOT NULL CHECK (length(source_table) BETWEEN 1 AND 120),
  source_key TEXT NOT NULL CHECK (length(source_key) BETWEEN 1 AND 500),
  source_created_at_utc TEXT,
  record_sha256 TEXT NOT NULL CHECK (length(record_sha256)=64),
  chain_hash TEXT NOT NULL CHECK (length(chain_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (archive_id) REFERENCES evidence_archives(archive_id) ON DELETE RESTRICT,
  UNIQUE (archive_id, ordinal),
  UNIQUE (source_table, source_key)
);

CREATE INDEX IF NOT EXISTS idx_evidence_archive_members_source
  ON evidence_archive_members (source_table, source_key);

CREATE TRIGGER IF NOT EXISTS trg_evidence_archive_members_no_update
BEFORE UPDATE ON evidence_archive_members
BEGIN
  SELECT RAISE(ABORT,'evidence archive members are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_evidence_archive_members_no_delete
BEFORE DELETE ON evidence_archive_members
BEGIN
  SELECT RAISE(ABORT,'evidence archive members are immutable');
END;

CREATE TABLE IF NOT EXISTS evidence_archive_exports (
  export_id TEXT PRIMARY KEY,
  archive_id TEXT NOT NULL,
  package_sha256 TEXT NOT NULL CHECK (length(package_sha256)=64),
  destination_file_name TEXT NOT NULL CHECK (length(destination_file_name) BETWEEN 1 AND 220),
  exported_by_user_id TEXT NOT NULL CHECK (length(exported_by_user_id) BETWEEN 1 AND 160),
  export_receipt_hash TEXT NOT NULL UNIQUE CHECK (length(export_receipt_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (archive_id) REFERENCES evidence_archives(archive_id) ON DELETE RESTRICT
);

CREATE TRIGGER IF NOT EXISTS trg_evidence_archive_exports_no_update
BEFORE UPDATE ON evidence_archive_exports
BEGIN
  SELECT RAISE(ABORT,'evidence archive export receipts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_evidence_archive_exports_no_delete
BEFORE DELETE ON evidence_archive_exports
BEGIN
  SELECT RAISE(ABORT,'evidence archive export receipts are immutable');
END;

CREATE TABLE IF NOT EXISTS evidence_archive_events (
  event_id TEXT PRIMARY KEY,
  archive_id TEXT,
  policy_id TEXT,
  event_type TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 120),
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error','denied')),
  actor_type TEXT NOT NULL CHECK (actor_type IN ('local_user','system')),
  actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 160),
  detail_code TEXT NOT NULL CHECK (length(detail_code) BETWEEN 1 AND 120),
  metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
  event_hash TEXT NOT NULL UNIQUE CHECK (length(event_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (archive_id) REFERENCES evidence_archives(archive_id) ON DELETE SET NULL,
  FOREIGN KEY (policy_id) REFERENCES evidence_archive_policies(policy_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_evidence_archive_events_recent
  ON evidence_archive_events (created_at_utc DESC,event_id DESC);

CREATE TRIGGER IF NOT EXISTS trg_evidence_archive_events_no_update
BEFORE UPDATE ON evidence_archive_events
BEGIN
  SELECT RAISE(ABORT,'evidence archive events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS trg_evidence_archive_events_no_delete
BEFORE DELETE ON evidence_archive_events
BEGIN
  SELECT RAISE(ABORT,'evidence archive events are append-only');
END;

INSERT OR IGNORE INTO evidence_archive_policies (
  policy_id,policy_revision,enabled,interval_hours,max_records_per_archive,
  retention_count,max_package_bytes,policy_hash,created_by_user_id,reason,created_at_utc
) VALUES (
  '00000000-0000-4000-8000-000000000029',1,1,24,5000,30,67108864,
  '610b795bf96f1b5a42962f2931f320034974b6cf294945d1e9c44a78159ecdf1',
  'system','Phase 21 safe local evidence archive default',
  strftime('%Y-%m-%dT%H:%M:%fZ','now')
);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0029_tamper_evident_evidence_archive');
