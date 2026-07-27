CREATE TABLE IF NOT EXISTS vault_semantic_documents (
    document_id TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK (state IN ('pending','indexing','ready','stale','failed')),
    embedding_model TEXT,
    source_sha256 TEXT NOT NULL CHECK (length(source_sha256) = 64),
    chunk_count INTEGER NOT NULL DEFAULT 0 CHECK (chunk_count >= 0),
    dimensions INTEGER NOT NULL DEFAULT 0 CHECK (dimensions >= 0),
    failure_code TEXT,
    embedded_at_utc TEXT,
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    FOREIGN KEY(document_id) REFERENCES vault_documents(document_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_vault_semantic_documents_state
    ON vault_semantic_documents(state, updated_at_utc DESC);

CREATE TABLE IF NOT EXISTS vault_semantic_chunks (
    chunk_id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    chunk_ordinal INTEGER NOT NULL CHECK (chunk_ordinal >= 0),
    page_number INTEGER CHECK (page_number IS NULL OR page_number > 0),
    chunk_text TEXT NOT NULL,
    chunk_sha256 TEXT NOT NULL CHECK (length(chunk_sha256) = 64),
    embedding_model TEXT NOT NULL,
    dimensions INTEGER NOT NULL CHECK (dimensions > 0 AND dimensions <= 4096),
    embedding_json TEXT NOT NULL,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(document_id, chunk_ordinal),
    FOREIGN KEY(document_id) REFERENCES vault_documents(document_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_vault_semantic_chunks_document
    ON vault_semantic_chunks(document_id, chunk_ordinal);
CREATE INDEX IF NOT EXISTS idx_vault_semantic_chunks_model
    ON vault_semantic_chunks(embedding_model, document_id);

CREATE TABLE IF NOT EXISTS vault_semantic_operations (
    operation_id TEXT PRIMARY KEY,
    operation_type TEXT NOT NULL CHECK (operation_type IN ('rebuild')),
    state TEXT NOT NULL CHECK (state IN ('pending','running','completed','failed','interrupted')),
    embedding_model TEXT NOT NULL,
    status_message TEXT NOT NULL,
    processed_documents INTEGER NOT NULL DEFAULT 0 CHECK (processed_documents >= 0),
    total_documents INTEGER NOT NULL DEFAULT 0 CHECK (total_documents >= 0),
    processed_chunks INTEGER NOT NULL DEFAULT 0 CHECK (processed_chunks >= 0),
    failed_documents INTEGER NOT NULL DEFAULT 0 CHECK (failed_documents >= 0),
    failure_code TEXT,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    completed_at_utc TEXT
);

CREATE INDEX IF NOT EXISTS idx_vault_semantic_operations_updated
    ON vault_semantic_operations(updated_at_utc DESC, operation_id DESC);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0007_semantic_vault');
