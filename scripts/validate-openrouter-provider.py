#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def require(source: str, marker: str, label: str) -> None:
    if marker not in source:
        raise SystemExit(f"OpenRouter validation failed: missing {label}: {marker}")


def forbid(source: str, marker: str, label: str) -> None:
    if marker in source:
        raise SystemExit(f"OpenRouter validation failed: forbidden {label}: {marker}")


migration = read("database/migrations/0016_openrouter_model_provider.sql")
provider = read("crates/homeserver-service/src/openrouter_provider.rs")
agent = read("crates/homeserver-service/src/agent_runtime.rs")
app = read("crates/homeserver-service/src/app.rs")
main = read("crates/homeserver-service/src/main.rs")
tauri = read("src-tauri/src/openrouter.rs")
tauri_lib = read("src-tauri/src/lib.rs")
frontend = read("src/openrouter-provider.js")
package = read("package.json")

for marker in [
    "CREATE TABLE IF NOT EXISTS model_provider_settings",
    "CREATE TABLE IF NOT EXISTS model_provider_usage_receipts",
    "0016_openrouter_model_provider",
    "credential_key TEXT NOT NULL",
]:
    require(migration, marker, "migration contract")

for marker in ["api_key TEXT", "prompt TEXT", "response TEXT", "conversation TEXT"]:
    forbid(migration.lower(), marker.lower(), "plaintext sensitive storage")

for marker in [
    'const API_BASE: &str = "https://openrouter.ai/api/v1"',
    "Policy::none()",
    "Entry::new(CREDENTIAL_SERVICE",
    '"/v1/models/providers/openrouter/catalog"',
    '"/v1/models/providers/openrouter/configure"',
    '"/v1/models/providers/openrouter/test"',
    '"/v1/models/providers/openrouter/disconnect"',
    'request.remote_context_confirmation.as_deref() == Some("SEND REMOTE")',
    'request.confirmation != "TEST REMOTE"',
    'request.confirmation != "DISCONNECT"',
    '"data_collection": settings.data_collection',
    '"zdr": settings.zdr_only',
    "model_provider_usage_receipts",
    "generate_agent_response",
]:
    require(provider, marker, "provider boundary")

for marker in ["OPENROUTER_API_KEY", "sk-or-v1-", "api_key TEXT", "prompt_contents", "conversation_contents"]:
    forbid(provider, marker, "embedded credential or private content")

for marker in [
    "openrouter_provider::initialize",
    "openrouter_provider::router",
    "openrouter_provider::maintain_history",
]:
    require(app, marker, "service wiring")

require(main, "mod openrouter_provider;", "module registration")
require(main, "openrouter_provider::health_check", "health check")
require(agent, "openrouter_provider::generate_agent_response", "Agent Workspace routing")
require(agent, '"openrouter_model_opt_in"', "Agent Workspace capability")

for marker in [
    "homeserver_openrouter_status",
    "homeserver_openrouter_catalog",
    "homeserver_configure_openrouter",
    "homeserver_test_openrouter",
    "homeserver_disconnect_openrouter",
]:
    require(tauri, marker, "Tauri provider command")
    require(tauri_lib, marker, "Tauri command registration")

for marker in [
    'invoke("homeserver_openrouter_status")',
    'invoke("homeserver_openrouter_catalog")',
    'invoke("homeserver_configure_openrouter"',
    'invoke("homeserver_test_openrouter"',
    'invoke("homeserver_disconnect_openrouter"',
    "SEND REMOTE",
    "TEST REMOTE",
    "Windows Credential Manager",
    'window.addEventListener("homeserver:rendered", mount)',
]:
    require(frontend, marker, "Control Center provider UI")

forbid(frontend, "MutationObserver", "app-wide observer")
require(package, "validate-openrouter-provider.py", "permanent frontend validator")
require(package, "node --check src/openrouter-provider.js", "frontend syntax gate")

print("OpenRouter provider validation passed")
