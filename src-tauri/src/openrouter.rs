use super::{get_json, post_json};
use serde_json::{json, Value};

#[tauri::command]
pub(crate) async fn homeserver_openrouter_status() -> Result<Value, String> {
    get_json("/v1/models/providers/openrouter").await
}

#[tauri::command]
pub(crate) async fn homeserver_openrouter_catalog() -> Result<Value, String> {
    get_json("/v1/models/providers/openrouter/catalog").await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub(crate) async fn homeserver_configure_openrouter(
    api_key: Option<String>,
    enabled: bool,
    allow_remote_context: bool,
    remote_context_confirmation: Option<String>,
    default_model: Option<String>,
    fallback_models: Vec<String>,
    monthly_budget_microusd: Option<u64>,
    monthly_request_limit: Option<u64>,
    max_output_tokens: u32,
    routing_sort: String,
    allow_provider_fallbacks: bool,
    data_collection: String,
    zdr_only: bool,
) -> Result<Value, String> {
    post_json(
        "/v1/models/providers/openrouter/configure",
        &json!({
            "api_key": api_key,
            "enabled": enabled,
            "allow_remote_context": allow_remote_context,
            "remote_context_confirmation": remote_context_confirmation,
            "default_model": default_model,
            "fallback_models": fallback_models,
            "monthly_budget_microusd": monthly_budget_microusd,
            "monthly_request_limit": monthly_request_limit,
            "max_output_tokens": max_output_tokens,
            "routing_sort": routing_sort,
            "allow_provider_fallbacks": allow_provider_fallbacks,
            "data_collection": data_collection,
            "zdr_only": zdr_only
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_test_openrouter(
    model: Option<String>,
    prompt: String,
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/models/providers/openrouter/test",
        &json!({
            "model": model,
            "prompt": prompt,
            "confirmation": confirmation
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_disconnect_openrouter(
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/models/providers/openrouter/disconnect",
        &json!({ "confirmation": confirmation }),
    )
    .await
}
