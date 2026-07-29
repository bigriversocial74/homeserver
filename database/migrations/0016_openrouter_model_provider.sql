-- Optional OpenRouter model provider for HomeServer.
-- API credentials remain in the operating-system credential vault and never enter SQLite.

CREATE TABLE IF NOT EXISTS model_provider_settings (
    provider_key TEXT PRIMARY KEY CHECK (provider_key IN ('openrouter')),
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0,1)),
    allow_remote_context INTEGER NOT NULL DEFAULT 0 CHECK (allow_remote_context IN (0,1)),
    default_model TEXT,
    fallback_models_json TEXT NOT NULL DEFAULT '[]',
    monthly_budget_microusd INTEGER CHECK (monthly_budget_microusd IS NULL OR monthly_budget_microusd >= 0),
    monthly_request_limit INTEGER CHECK (monthly_request_limit IS NULL OR monthly_request_limit >= 1),
    max_output_tokens INTEGER NOT NULL DEFAULT 800 CHECK (max_output_tokens BETWEEN 16 AND 4096),
    routing_sort TEXT NOT NULL DEFAULT 'price' CHECK (routing_sort IN ('price','throughput','latency')),
    allow_provider_fallbacks INTEGER NOT NULL DEFAULT 1 CHECK (allow_provider_fallbacks IN (0,1)),
    data_collection TEXT NOT NULL DEFAULT 'deny' CHECK (data_collection IN ('allow','deny')),
    zdr_only INTEGER NOT NULL DEFAULT 0 CHECK (zdr_only IN (0,1)),
    credential_key TEXT NOT NULL,
    last_tested_at_utc TEXT,
    last_success_at_utc TEXT,
    last_error_code TEXT,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT OR IGNORE INTO model_provider_settings (
    provider_key,
    credential_key
) VALUES (
    'openrouter',
    'model-provider:openrouter:api-key'
);

CREATE TABLE IF NOT EXISTS model_provider_usage_receipts (
    receipt_id TEXT PRIMARY KEY,
    provider_key TEXT NOT NULL CHECK (provider_key IN ('openrouter')),
    request_id TEXT,
    request_kind TEXT NOT NULL CHECK (request_kind IN ('connection_test','agent_prompt','manual_test')),
    requested_model TEXT NOT NULL,
    resolved_model TEXT,
    prompt_tokens INTEGER NOT NULL DEFAULT 0 CHECK (prompt_tokens >= 0),
    completion_tokens INTEGER NOT NULL DEFAULT 0 CHECK (completion_tokens >= 0),
    total_tokens INTEGER NOT NULL DEFAULT 0 CHECK (total_tokens >= 0),
    reported_cost_microusd INTEGER NOT NULL DEFAULT 0 CHECK (reported_cost_microusd >= 0),
    state TEXT NOT NULL CHECK (state IN ('succeeded','failed','blocked')),
    error_code TEXT,
    duration_ms INTEGER NOT NULL DEFAULT 0 CHECK (duration_ms >= 0),
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_model_provider_usage_month
    ON model_provider_usage_receipts(provider_key, created_at_utc DESC, receipt_id DESC);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0016_openrouter_model_provider');
