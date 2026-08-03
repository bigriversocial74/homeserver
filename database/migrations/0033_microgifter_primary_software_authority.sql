-- Restore Microgifter as the primary HomeServer account, entitlement and update authority.
--
-- This migration is intentionally additive. It preserves the historical VP3 authority
-- row, device identifiers, receipts and optional wrapper/provider records so installed
-- systems retain their complete audit history and can still use VP3 as an optional
-- paired service. No HomeServer identity, credential, grant or local data is replaced.

CREATE TABLE IF NOT EXISTS homeserver_primary_authority (
  singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
  provider_key TEXT NOT NULL DEFAULT 'microgifter' CHECK (provider_key = 'microgifter'),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','error')),
  migrated_from TEXT CHECK (migrated_from IS NULL OR migrated_from IN ('vp3','microgifter_legacy')),
  restored_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT OR IGNORE INTO homeserver_primary_authority (
  singleton_id,
  provider_key,
  state,
  migrated_from
) VALUES (
  1,
  'microgifter',
  'active',
  CASE
    WHEN EXISTS (
      SELECT 1
      FROM homeserver_software_authority
      WHERE singleton_id = 1
        AND (current_authority = 'vp3' OR target_authority = 'vp3')
    ) THEN 'vp3'
    ELSE 'microgifter_legacy'
  END
);

UPDATE homeserver_primary_authority
SET provider_key = 'microgifter',
    state = 'active',
    updated_at_utc = strftime('%Y-%m-%dT%H:%M:%fZ','now')
WHERE singleton_id = 1;

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0033_microgifter_primary_software_authority');
