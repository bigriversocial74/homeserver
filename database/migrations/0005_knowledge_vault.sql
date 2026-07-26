
CREATE TABLE IF NOT EXISTS vault_documents (
    document_id TEXT PRIMARY KEY,
    file_name TEXT NOT NULL,
    title TEXT NOT NULL,
    managed_path TEXT NOT NULL UNIQUE,
    content_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256 TEXT NOT NULL UNIQUE CHECK (length(sha256) = 64),
    state TEXT NOT NULL CHECK (state IN ('indexed','changed','missing','failed')),
    tags_json TEXT NOT NULL DEFAULT '[]',
    indexed_text TEXT NOT NULL DEFAULT '',
    failure_code TEXT,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    indexed_at_utc TEXT
);

CREATE INDEX IF NOT EXISTS idx_vault_documents_state_updated
    ON vault_documents(state, updated_at_utc DESC);
CREATE INDEX IF NOT EXISTS idx_vault_documents_title
    ON vault_documents(title);

CREATE TABLE IF NOT EXISTS vault_access_rules (
    access_rule_id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    access_level TEXT NOT NULL CHECK (access_level IN ('read','search')),
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(agent_id, document_id, access_level),
    FOREIGN KEY(document_id) REFERENCES vault_documents(document_id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0005_knowledge_vault');
