#!/usr/bin/env python3
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    target = ROOT / path
    if not target.is_file():
        raise AssertionError(f"missing required Phase 20 file: {path}")
    return target.read_text(encoding="utf-8")


def require(text: str, needles: list[str], label: str) -> None:
    missing = [needle for needle in needles if needle not in text]
    if missing:
        raise AssertionError(f"{label} is missing: {', '.join(missing)}")


def forbid(text: str, needles: list[str], label: str) -> None:
    present = [needle for needle in needles if needle in text]
    if present:
        raise AssertionError(f"{label} contains forbidden content: {', '.join(present)}")


def main() -> int:
    migration = read("database/migrations/0028_authorized_model_routing.sql")
    service = read("crates/homeserver-service/src/inference_governance.rs")
    openrouter = read("crates/homeserver-service/src/openrouter_provider.rs")
    agent = read("crates/homeserver-service/src/agent_runtime.rs")
    app = read("crates/homeserver-service/src/app.rs")
    main_rs = read("crates/homeserver-service/src/main.rs")
    tauri_runtime = read("src-tauri/src/runtime.rs")
    tauri_lib = read("src-tauri/src/lib.rs")
    frontend = read("src/agent-runtime-control-center.js")
    package = read("package.json")
    docs = read("docs/phase-20-authorized-model-routing-inference-governance.md")
    workflow = read(".github/workflows/phase20-model-inference-governance.yml")
    tests = read("crates/homeserver-service/tests/phase20_model_inference_governance_contract.rs")

    require(
        migration,
        [
            "CREATE TABLE IF NOT EXISTS model_routing_policies",
            "CREATE TABLE IF NOT EXISTS model_inference_requests",
            "CREATE TABLE IF NOT EXISTS model_inference_attempts",
            "CREATE TABLE IF NOT EXISTS model_inference_private_results",
            "CREATE TABLE IF NOT EXISTS model_inference_receipts",
            "CREATE TABLE IF NOT EXISTS model_inference_events",
            "trg_model_routing_policy_authority_immutable",
            "trg_model_inference_request_terminal_immutable",
            "trg_model_inference_private_result_no_delete",
            "trg_model_inference_receipt_no_delete",
            "trg_model_inference_event_no_delete",
            "0028_authorized_model_routing",
            "[\"ollama\"]",
            "'deny'",
        ],
        "Phase 20 migration",
    )
    forbid(migration, ["sha3(", "ON DELETE CASCADE\n);\n\nCREATE TRIGGER IF NOT EXISTS trg_model_inference_receipt"], "Phase 20 migration")

    require(
        service,
        [
            "homeserver.model-inference-governance.v1",
            "homeserver.model-inference-authority.v1",
            "homeserver.model-routing-decision.v1",
            "silent_remote_fallback_allowed: false",
            "provider_can_grant_authority: false",
            "private_prompts_exposed: false",
            "private_results_exposed: false",
            "revalidate_authority",
            "validate_remote_context",
            "ensure_no_emergency_stop",
            "enforce_agent_model_restrictions",
            "remote_model_mode='approved_provider'",
            "approved_remote_provider='openrouter'",
            "private_source",
            "policy_usage",
            "idempotency key was reused with a different request",
            "generate_governed_response",
            "model_center::generate_text",
            "finalize_unreceipted_terminal_requests_tx",
        ],
        "Phase 20 service",
    )
    forbid(
        service,
        [
            "Command::new",
            "powershell",
            "cmd.exe",
            "http://0.0.0.0",
            "unwrap_unchecked",
            "private_prompts_exposed: true",
            "private_results_exposed: true",
            "silent_remote_fallback_allowed: true",
        ],
        "Phase 20 service",
    )

    require(
        openrouter,
        [
            "generate_governed_response",
            "snapshot_from_connection_for_governance",
            "allow_configured_fallbacks",
            "max_output_tokens_override",
        ],
        "OpenRouter governed adapter",
    )
    require(
        agent,
        [
            "inference_governance::infer",
            "inference_policy_id",
            "data_classification",
            "provider_preference",
            "privacy_selector_id",
            "inference_idempotency_key",
            "pub inference: Option<inference_governance::GovernedInferenceResult>",
        ],
        "Agent Workspace governed routing",
    )
    forbid(
        agent,
        [
            "OpenRouter default model failed; trying local model",
            "openrouter_provider::generate_agent_response(",
        ],
        "Agent Workspace routing",
    )

    require(
        app,
        [
            "inference_governance::initialize(&connection)?",
            ".merge(inference_governance::router(state.clone()))",
            "inference_governance::maintain_history(&connection)",
        ],
        "service lifecycle integration",
    )
    require(
        main_rs,
        [
            "mod inference_governance;",
            "inference_governance::health_check(&connection)",
            "inference_governance::maintain_history(&connection)?",
        ],
        "service health integration",
    )

    require(
        tauri_runtime,
        [
            "homeserver_model_governance",
            "homeserver_create_model_policy",
            "homeserver_revoke_model_policy",
            "homeserver_cancel_model_inference",
            "/v1/models/governance",
        ],
        "trusted desktop bridge",
    )
    require(
        tauri_lib,
        [
            "runtime::homeserver_model_governance",
            "runtime::homeserver_create_model_policy",
            "runtime::homeserver_revoke_model_policy",
            "runtime::homeserver_cancel_model_inference",
        ],
        "Tauri command registration",
    )
    require(
        frontend,
        [
            "homeserver_model_governance",
            "Model Inference Governance",
            "private_prompts_exposed",
            "silent_remote_fallback_allowed",
            "data-model-policy-revoke",
            "data-model-inference-cancel",
        ],
        "Agent Runtime governance UI",
    )
    forbid(frontend, ["MutationObserver", "output_text", "private_results"], "Agent Runtime governance UI")

    require(package, ["validate-model-inference-governance.py"], "frontend validation registration")
    require(
        workflow,
        [
            "Phase 20 Authorized Model Routing and Inference Governance",
            "validate-model-inference-governance.py",
            "phase20_model_inference_governance_contract",
            "cargo clippy -p microgifter-homeserver-service --all-targets -- -D warnings",
        ],
        "Phase 20 exact-head workflow",
    )
    require(tests, ["terminal_requests_attempts_results_receipts_and_events_are_immutable", "default_policy_is_local_only_and_remote_denied"], "Phase 20 native tests")
    require(docs, ["Initial audit: **6.2/10**", "No Microgifter, VP3, or POD MySQL import is required"], "Phase 20 documentation")

    print("Phase 20 authorized model routing and inference governance contract passed.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except AssertionError as error:
        print(f"Phase 20 contract failed: {error}", file=sys.stderr)
        raise SystemExit(1)
