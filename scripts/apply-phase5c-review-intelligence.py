#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, value: str) -> None:
    (ROOT / path).write_text(value, encoding="utf-8", newline="\n")


def replace_once(value: str, old: str, new: str, label: str) -> str:
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, found {count}")
    return value.replace(old, new, 1)

# Main service module, health, and retention integration.
main = read("crates/homeserver-service/src/main.rs")
main = replace_once(main, "mod recovery_transfer;\n", "mod recovery_transfer;\nmod review_intelligence;\n", "main module")
main = replace_once(
    main,
    """        if let Err(error) = agent_runtime::health_check(&connection) {
""",
    """        if let Err(error) = review_intelligence::health_check(&connection) {
            error!(
                ?error,
                "HomeServer review intelligence database health check failed"
            );
            return HealthSnapshot::needs_attention(
                &self.config.server_name,
                "review_intelligence_integrity_check_failed",
            );
        }
        if let Err(error) = agent_runtime::health_check(&connection) {
""",
    "main health check",
)
main = replace_once(
    main,
    """            operational_data::maintain_history(&connection)?;
            agent_runtime::maintain_history(&connection)?;
""",
    """            operational_data::maintain_history(&connection)?;
            review_intelligence::maintain_history(&connection)?;
            agent_runtime::maintain_history(&connection)?;
""",
    "main history",
)
write("crates/homeserver-service/src/main.rs", main)

app = read("crates/homeserver-service/src/app.rs")
app = replace_once(
    app,
    """    mcp_runtime, model_center, operational_data, semantic_vault, update, update_store, AppState,
""",
    """    mcp_runtime, model_center, operational_data, review_intelligence, semantic_vault, update,
    update_store, AppState,
""",
    "app import",
)
app = replace_once(
    app,
    """    model_center::initialize(&connection)?;
    semantic_vault::initialize(&connection)?;
    agent_runtime::initialize(&connection)?;
""",
    """    model_center::initialize(&connection)?;
    semantic_vault::initialize(&connection)?;
    review_intelligence::initialize(&connection)?;
    agent_runtime::initialize(&connection)?;
""",
    "app initialize",
)
app = replace_once(
    app,
    """            .merge(operational_data::router(state.clone()))
            .merge(agent_runtime::router(state.clone()))
""",
    """            .merge(operational_data::router(state.clone()))
            .merge(review_intelligence::router(state.clone()))
            .merge(agent_runtime::router(state.clone()))
""",
    "app router",
)
write("crates/homeserver-service/src/app.rs", app)

# Provider-neutral operational storage gains the installed Microgifter catalog
# and a provider-adapter import entry point.
operational = read("crates/homeserver-service/src/operational_data.rs")
uses = """const PERMITTED_AGENT_USES: &[&str] = &[
    "read",
    "analyze",
    "goal_match",
    "report",
    "sentiment_analysis",
    "semantic_clustering",
    "conversation_continuity",
    "commitment_extraction",
    "follow_up",
    "service_recovery",
    "relationship_management",
    "campaign_targeting",
    "campaign_management",
    "campaign_optimization",
    "customer_value",
    "product_affinity",
    "gifting_relationships",
    "consent_enforcement",
    "policy_enforcement",
    "supervised_planning",
];
"""
operational = re.sub(
    r'const PERMITTED_AGENT_USES: &\[&str\] = &\[[^;]+;\n',
    uses,
    operational,
    count=1,
    flags=re.S,
)
datasets = r'''const MICROGIFTER_DATASETS: &[BuiltinDataset] = &[
    BuiltinDataset { key: "merchant.profile", label: "Merchant Profile", description: "Provider-authoritative merchant identity and operating details.", sensitivity: "business", retention_days: 365 },
    BuiltinDataset { key: "merchant.locations", label: "Store Locations", description: "Locations, GPS coordinates, hours, and operating metadata.", sensitivity: "business", retention_days: 365 },
    BuiltinDataset { key: "merchant.products", label: "Products", description: "Products, pricing, availability, and catalog attributes.", sensitivity: "business", retention_days: 365 },
    BuiltinDataset { key: "merchant.inventory", label: "Inventory", description: "Product inventory, availability, and reservation state.", sensitivity: "business", retention_days: 180 },
    BuiltinDataset { key: "merchant.staff", label: "Merchant Staff", description: "Authorized staff, roles, assignments, and schedules.", sensitivity: "restricted", retention_days: 180 },
    BuiltinDataset { key: "merchant.store_activity", label: "Store Activity", description: "Provider-authoritative store and location activity evidence.", sensitivity: "business", retention_days: 365 },
    BuiltinDataset { key: "reviews.customer_reviews", label: "Customer Reviews", description: "Review text, ratings, context, and merchant response state.", sensitivity: "restricted", retention_days: 730 },
    BuiltinDataset { key: "reviews.resolution_history", label: "Review Resolution History", description: "Review responses, service recovery, and resolution events.", sensitivity: "restricted", retention_days: 730 },
    BuiltinDataset { key: "conversations.threads", label: "Conversation Threads", description: "Customer and merchant conversation state, intent, ownership, and closure.", sensitivity: "restricted", retention_days: 730 },
    BuiltinDataset { key: "conversations.messages", label: "Conversation Messages", description: "Authorized message bodies and delivery history for semantic understanding and continuity.", sensitivity: "sensitive", retention_days: 730 },
    BuiltinDataset { key: "conversations.follow_ups", label: "Conversation Follow-Ups", description: "Commitments, next steps, due dates, and closure evidence.", sensitivity: "restricted", retention_days: 730 },
    BuiltinDataset { key: "crm.contacts", label: "CRM Contacts", description: "Authorized customer identities, lifecycle, relationship, preference, and consent context.", sensitivity: "sensitive", retention_days: 730 },
    BuiltinDataset { key: "crm.activities", label: "CRM Activities", description: "Customer activity and relationship timeline evidence.", sensitivity: "restricted", retention_days: 730 },
    BuiltinDataset { key: "crm.tasks", label: "CRM Tasks", description: "Assigned CRM tasks, priorities, due dates, and completion state.", sensitivity: "restricted", retention_days: 365 },
    BuiltinDataset { key: "crm.notes", label: "CRM Notes", description: "Authorized merchant CRM notes and relationship context.", sensitivity: "sensitive", retention_days: 730 },
    BuiltinDataset { key: "crm.consent", label: "CRM Consent", description: "Provider-authoritative communication consent and preferences.", sensitivity: "sensitive", retention_days: 730 },
    BuiltinDataset { key: "commerce.orders", label: "Orders", description: "Orders, totals, status, location, campaign attribution, and purchase history without payment credentials.", sensitivity: "sensitive", retention_days: 1095 },
    BuiltinDataset { key: "commerce.order_items", label: "Order Items", description: "Products, quantities, prices, and product affinity evidence.", sensitivity: "sensitive", retention_days: 1095 },
    BuiltinDataset { key: "commerce.refunds", label: "Refunds", description: "Refund and service-recovery history without payment credentials.", sensitivity: "sensitive", retention_days: 1095 },
    BuiltinDataset { key: "gifts.ownership", label: "Gift and Wallet Ownership", description: "Provider-authoritative PPPM ownership and lifecycle copies for analysis.", sensitivity: "sensitive", retention_days: 1095 },
    BuiltinDataset { key: "gifts.claims", label: "Gift Claims", description: "Gift claim history and status evidence.", sensitivity: "sensitive", retention_days: 1095 },
    BuiltinDataset { key: "gifts.redemptions", label: "Gift Redemptions", description: "Gift redemption history and location evidence.", sensitivity: "sensitive", retention_days: 1095 },
    BuiltinDataset { key: "campaigns.definition", label: "Campaign Definitions", description: "Campaign type, rules, audience, dates, rewards, and lifecycle state.", sensitivity: "business", retention_days: 730 },
    BuiltinDataset { key: "campaigns.performance", label: "Campaign Performance", description: "Campaign events, delivery, claims, redemption, conversion, and CRM outcomes.", sensitivity: "restricted", retention_days: 1095 },
    BuiltinDataset { key: "campaigns.authorizations", label: "Agent Campaign Authorizations", description: "Merchant-owned provider policy for campaign analysis, drafting, approval, and execution.", sensitivity: "sensitive", retention_days: 730 },
    BuiltinDataset { key: "creator.attribution", label: "Creator Attribution", description: "Creator, referral, campaign, customer, and value attribution.", sensitivity: "restricted", retention_days: 1095 },
];
'''
operational = re.sub(
    r'const MICROGIFTER_DATASETS: &\[BuiltinDataset\] = &\[.*?\n\];\n',
    datasets,
    operational,
    count=1,
    flags=re.S,
)
operational = replace_once(
    operational,
    """fn import_operational_batch(
    state: &AppState,
""",
    """pub(crate) fn import_for_provider(
    state: &AppState,
    request: ImportOperationalBatchRequest,
) -> Result<ImportOperationalBatchResult> {
    import_operational_batch(state, request)
}

fn import_operational_batch(
    state: &AppState,
""",
    "operational provider import",
)
operational = operational.replace(
    '["read", "analyze", "goal_match", "report"].contains(&use_name.as_str())',
    'PERMITTED_AGENT_USES.contains(&use_name.as_str())',
)
write("crates/homeserver-service/src/operational_data.rs", operational)

# Fixed-path signed provider calls reuse the established device credentials.
cloud = read("crates/homeserver-service/src/cloud_registry.rs")
cloud = replace_once(
    cloud,
    'const SYNC_PATH: &str = "/api/homeserver/sync.php";\n',
    'const SYNC_PATH: &str = "/api/homeserver/sync.php";\nconst OPERATIONAL_MANIFEST_PATH: &str = "/api/homeserver/operational-manifest.php";\nconst OPERATIONAL_EXPORT_PATH: &str = "/api/homeserver/operational-export.php";\nconst CAMPAIGN_ACTIONS_PATH: &str = "/api/homeserver/campaign-actions.php";\n',
    "provider paths",
)
provider_helper = r'''
pub(crate) async fn provider_post_json(
    state: &AppState,
    connection_id: &str,
    path: &str,
    body: &Value,
) -> Result<Value> {
    validate_connection_id(connection_id)?;
    let required_scope = match path {
        OPERATIONAL_EXPORT_PATH | OPERATIONAL_MANIFEST_PATH => "homeserver.operational.read",
        CAMPAIGN_ACTIONS_PATH => "homeserver.campaigns.execute",
        _ => bail!("provider path is not installed"),
    };
    let record = connection_record(&*state.connection()?, connection_id)?;
    ensure!(
        !matches!(record.summary.state, CloudRegistryConnectionState::Revoked | CloudRegistryConnectionState::Disconnected),
        "cloud connection is inactive"
    );
    ensure!(
        record.summary.scopes.iter().any(|scope| scope == required_scope),
        "paired connection does not contain the required provider scope"
    );
    let secrets = load_secrets(&record.credential_key)?;
    let client = provider_client(&record.summary.provider_key)?;
    let body = canonical_json_string(body)?;
    match client
        .signed_request::<Value>(Method::POST, path, &body, &record, &secrets)
        .await
    {
        Ok(value) => {
            mark_connection_success(&*state.connection()?, connection_id)?;
            Ok(value)
        }
        Err(error) => {
            mark_connection_error(
                &*state.connection()?,
                connection_id,
                &public_cloud_error(&error),
                authentication_failed(&error),
            )?;
            Err(error)
        }
    }
}

fn canonical_json_string(value: &Value) -> Result<String> {
    fn canonicalize(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort();
                let mut result = serde_json::Map::new();
                for key in keys {
                    result.insert(key.clone(), canonicalize(&object[key]));
                }
                Value::Object(result)
            }
            Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
            _ => value.clone(),
        }
    }
    Ok(serde_json::to_string(&canonicalize(value))?)
}

'''
cloud = replace_once(cloud, "fn provider_client(provider_key: &str) -> Result<MicrogifterCloudClient> {\n", provider_helper + "fn provider_client(provider_key: &str) -> Result<MicrogifterCloudClient> {\n", "provider helper")
write("crates/homeserver-service/src/cloud_registry.rs", cloud)

# Bounded local model generation for semantic review analysis.
models = read("crates/homeserver-service/src/model_center.rs")
generate = r'''
pub(crate) async fn generate_text(
    state: Arc<AppState>,
    model: String,
    prompt: String,
    max_predict: u32,
) -> Result<String> {
    let definition = approved_model(&model)?;
    ensure!(definition.supports_chat, "configured model does not support chat generation");
    let prompt = prompt.trim();
    ensure!(!prompt.is_empty(), "model prompt is required");
    ensure!(prompt.chars().count() <= 30_000, "model prompt exceeds the 30000 character limit");
    let max_predict = max_predict.clamp(64, 4_096);
    let settings = {
        let state_for_settings = state.clone();
        tokio::task::spawn_blocking(move || read_settings(&state_for_settings))
            .await
            .context("model settings task failed")??
    };
    let client = ollama_client(settings.test_timeout_seconds.max(30))?;
    let response = client
        .post(format!("{OLLAMA_API_BASE}/api/generate"))
        .json(&serde_json::json!({
            "model": definition.model,
            "prompt": prompt,
            "stream": false,
            "keep_alive": 0,
            "format": "json",
            "options": {
                "num_ctx": settings.context_size,
                "num_predict": max_predict,
                "temperature": 0.2
            }
        }))
        .send()
        .await?;
    let payload: OllamaGenerateResponse = decode_json(response, MAX_OLLAMA_JSON_BYTES).await?;
    ensure!(!payload.response.trim().is_empty(), "model returned an empty response");
    Ok(payload.response.trim().chars().take(40_000).collect())
}

'''
models = replace_once(models, "fn local_snapshot(state: &AppState) -> Result<LocalSnapshot> {\n", generate + "fn local_snapshot(state: &AppState) -> Result<LocalSnapshot> {\n", "model generation")
write("crates/homeserver-service/src/model_center.rs", models)

# Campaign actions are proposed and executed through the existing local,
# one-use, hash-bound approval engine.
agent = read("crates/homeserver-service/src/agent_runtime.rs")
agent = replace_once(
    agent,
    "use crate::{app::cloud_registry, model_center, operational_data, semantic_vault, AppState};\n",
    "use crate::{\n    app::cloud_registry, model_center, operational_data, review_intelligence, semantic_vault,\n    AppState,\n};\n",
    "agent import",
)
agent = replace_once(
    agent,
    '    "report.save",\n];\n',
    '    "report.save",\n    "campaign.draft",\n    "campaign.publish",\n    "campaign.pause",\n    "campaign.resume",\n    "campaign.send_make_good",\n    "campaign.send_authorized",\n];\n',
    "agent actions",
)
agent = replace_once(
    agent,
    """        "report.save" => {
""",
    """        "campaign.draft"
        | "campaign.publish"
        | "campaign.pause"
        | "campaign.resume"
        | "campaign.send_make_good"
        | "campaign.send_authorized" => {
            review_intelligence::execute_campaign_plan(state, plan).await
        }
        "report.save" => {
""",
    "campaign executor",
)
write("crates/homeserver-service/src/agent_runtime.rs", agent)

print("HomeServer review intelligence backend integration applied.")
