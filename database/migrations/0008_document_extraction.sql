CREATE TABLE IF NOT EXISTS vault_document_extractions (
    document_id TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK (state IN ('ready','partial','ocr_required','failed')),
    extraction_method TEXT NOT NULL,
    source_sha256 TEXT NOT NULL CHECK (length(source_sha256) = 64),
    page_count INTEGER NOT NULL DEFAULT 0 CHECK (page_count >= 0 AND page_count <= 200),
    native_page_count INTEGER NOT NULL DEFAULT 0 CHECK (native_page_count >= 0),
    ocr_page_count INTEGER NOT NULL DEFAULT 0 CHECK (ocr_page_count >= 0),
    ocr_required_page_count INTEGER NOT NULL DEFAULT 0 CHECK (ocr_required_page_count >= 0),
    extracted_char_count INTEGER NOT NULL DEFAULT 0 CHECK (extracted_char_count >= 0),
    confidence_permille INTEGER CHECK (confidence_permille IS NULL OR confidence_permille BETWEEN 0 AND 1000),
    failure_code TEXT,
    extracted_at_utc TEXT,
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    FOREIGN KEY(document_id) REFERENCES vault_documents(document_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_vault_document_extractions_state
    ON vault_document_extractions(state, updated_at_utc DESC);

CREATE TABLE IF NOT EXISTS vault_document_pages (
    document_id TEXT NOT NULL,
    page_number INTEGER NOT NULL CHECK (page_number > 0 AND page_number <= 200),
    extraction_method TEXT NOT NULL,
    page_text TEXT NOT NULL DEFAULT '',
    page_text_sha256 TEXT NOT NULL CHECK (length(page_text_sha256) = 64),
    confidence_permille INTEGER CHECK (confidence_permille IS NULL OR confidence_permille BETWEEN 0 AND 1000),
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    PRIMARY KEY(document_id, page_number),
    FOREIGN KEY(document_id) REFERENCES vault_documents(document_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_vault_document_pages_document
    ON vault_document_pages(document_id, page_number);

CREATE TABLE IF NOT EXISTS vault_extraction_operations (
    operation_id TEXT PRIMARY KEY,
    document_id TEXT,
    file_name TEXT NOT NULL,
    operation_type TEXT NOT NULL CHECK (operation_type IN ('import','reindex')),
    state TEXT NOT NULL CHECK (state IN ('pending','running','completed','failed','interrupted')),
    status_message TEXT NOT NULL,
    processed_pages INTEGER NOT NULL DEFAULT 0 CHECK (processed_pages >= 0),
    total_pages INTEGER NOT NULL DEFAULT 0 CHECK (total_pages >= 0 AND total_pages <= 200),
    failure_code TEXT,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    completed_at_utc TEXT,
    FOREIGN KEY(document_id) REFERENCES vault_documents(document_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_vault_extraction_operations_updated
    ON vault_extraction_operations(updated_at_utc DESC, operation_id DESC);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0008_document_extraction');
