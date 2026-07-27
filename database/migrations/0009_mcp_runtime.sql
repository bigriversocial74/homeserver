CREATE TABLE IF NOT EXISTS mcp_clients (
    client_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 3 AND 80),
    token_hash TEXT NOT NULL UNIQUE CHECK (length(token_hash) = 64),
    token_hint TEXT NOT NULL CHECK (length(token_hint) BETWEEN 8 AND 32),
    scopes_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('active','revoked')),
    expires_at_utc TEXT NOT NULL,
    last_used_at_utc TEXT,
    request_count INTEGER NOT NULL DEFAULT 0 CHECK (request_count >= 0),
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    revoked_at_utc TEXT
);

CREATE INDEX IF NOT EXISTS idx_mcp_clients_state_expiry
    ON mcp_clients(state, expires_at_utc, updated_at_utc DESC);

CREATE TABLE IF NOT EXISTS mcp_rate_limits (
    client_id TEXT NOT NULL,
    window_epoch_minute INTEGER NOT NULL,
    request_count INTEGER NOT NULL DEFAULT 0 CHECK (request_count >= 0),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    PRIMARY KEY(client_id, window_epoch_minute),
    FOREIGN KEY(client_id) REFERENCES mcp_clients(client_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS mcp_audit_receipts (
    receipt_id TEXT PRIMARY KEY,
    client_id TEXT,
    method TEXT NOT NULL,
    capability TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('success','denied','error')),
    detail_code TEXT NOT NULL,
    request_bytes INTEGER NOT NULL CHECK (request_bytes >= 0),
    response_bytes INTEGER NOT NULL CHECK (response_bytes >= 0),
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    FOREIGN KEY(client_id) REFERENCES mcp_clients(client_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_mcp_audit_receipts_created
    ON mcp_audit_receipts(created_at_utc DESC, receipt_id DESC);
CREATE INDEX IF NOT EXISTS idx_mcp_audit_receipts_client
    ON mcp_audit_receipts(client_id, created_at_utc DESC);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0009_mcp_runtime');
