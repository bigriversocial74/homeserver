PRAGMA foreign_keys = OFF;

BEGIN IMMEDIATE;

ALTER TABLE update_records RENAME TO update_records_legacy_0002;

CREATE TABLE update_runtime (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    state TEXT NOT NULL CHECK (state IN ('idle','checking','current','available','downloading','staged','applying','succeeded','failed','rolled_back')),
    last_checked_at_utc TEXT,
    last_error TEXT,
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT INTO update_runtime (singleton_id, state)
VALUES (1, 'idle');

CREATE TABLE update_records (
    update_id TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    channel TEXT NOT NULL CHECK (channel IN ('stable')),
    state TEXT NOT NULL CHECK (state IN ('available','downloading','staged','applying','succeeded','failed','rolled_back')),
    manifest_url TEXT NOT NULL,
    manifest_json TEXT NOT NULL,
    release_notes TEXT NOT NULL DEFAULT '',
    installer_url TEXT NOT NULL,
    installer_file_name TEXT NOT NULL,
    installer_path TEXT,
    installer_size_bytes INTEGER NOT NULL CHECK (installer_size_bytes >= 0),
    installer_sha256 TEXT NOT NULL,
    authenticode_thumbprint TEXT NOT NULL,
    checked_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    downloaded_at_utc TEXT,
    applied_at_utc TEXT,
    failure_code TEXT,
    rollback_path TEXT,
    pre_update_backup_id TEXT,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    FOREIGN KEY (pre_update_backup_id) REFERENCES backup_records (backup_id) ON DELETE RESTRICT
);

INSERT INTO update_records (
    update_id,
    version,
    channel,
    state,
    manifest_url,
    manifest_json,
    release_notes,
    installer_url,
    installer_file_name,
    installer_path,
    installer_size_bytes,
    installer_sha256,
    authenticode_thumbprint,
    checked_at_utc,
    downloaded_at_utc,
    applied_at_utc,
    failure_code,
    rollback_path,
    pre_update_backup_id,
    created_at_utc,
    updated_at_utc
)
SELECT
    update_id,
    version,
    'stable',
    'failed',
    COALESCE(NULLIF(manifest_url, ''), 'https://invalid.local/legacy'),
    json_object(
        'key_id', 'legacy-untrusted',
        'payload', json_object(
            'schema_version', 1,
            'product', 'Microgifter HomeServer',
            'channel', 'stable',
            'version', version,
            'minimum_version', NULL,
            'published_at_utc', created_at_utc,
            'release_notes', 'Legacy unsigned update metadata preserved for audit only.',
            'installer', json_object(
                'url', 'https://invalid.local/legacy',
                'file_name', 'Microgifter-HomeServer-Setup.exe',
                'size_bytes', 0,
                'sha256', CASE
                    WHEN length(installer_sha256) = 64 THEN lower(installer_sha256)
                    ELSE '0000000000000000000000000000000000000000000000000000000000000000'
                END,
                'authenticode_thumbprint', '0000000000000000000000000000000000000000'
            )
        ),
        'signature', COALESCE(manifest_signature, '')
    ),
    'Legacy unsigned update metadata preserved for audit only.',
    'https://invalid.local/legacy',
    'Microgifter-HomeServer-Setup.exe',
    installer_path,
    0,
    CASE
        WHEN length(installer_sha256) = 64 THEN lower(installer_sha256)
        ELSE '0000000000000000000000000000000000000000000000000000000000000000'
    END,
    '0000000000000000000000000000000000000000',
    created_at_utc,
    CASE WHEN installer_path IS NOT NULL THEN created_at_utc ELSE NULL END,
    NULL,
    'legacy_unsigned_update_record',
    NULL,
    pre_update_backup_id,
    created_at_utc,
    updated_at_utc
FROM update_records_legacy_0002;

DROP TABLE update_records_legacy_0002;

CREATE INDEX idx_update_records_state_version
    ON update_records (state, version, updated_at_utc DESC);

CREATE TABLE update_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    update_id TEXT,
    event_type TEXT NOT NULL,
    message TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    FOREIGN KEY (update_id) REFERENCES update_records(update_id) ON DELETE SET NULL
);

CREATE INDEX idx_update_events_update_created
    ON update_events (update_id, created_at_utc DESC);

INSERT OR IGNORE INTO homeserver_settings (setting_key, setting_value)
VALUES ('update_channel', 'stable');

INSERT INTO schema_migrations (migration_key)
VALUES ('0003_signed_updates');

COMMIT;

PRAGMA foreign_keys = ON;
