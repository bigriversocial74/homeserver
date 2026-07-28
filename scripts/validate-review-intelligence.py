#!/usr/bin/env python3
"""Validate deterministic review intelligence, optional LLM, and campaign authority boundaries."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ERRORS: list[str] = []


def read(path: str) -> str:
    target = ROOT / path
    if not target.is_file():
        ERRORS.append(f"required review intelligence file is missing: {path}")
        return ""
    return target.read_text(encoding="utf-8")


def require(path: str, marker: str, message: str) -> None:
    if marker not in read(path):
        ERRORS.append(message)


def forbid(path: str, marker: str, message: str) -> None:
    if marker in read(path):
        ERRORS.append(message)


MIGRATION = "database/migrations/0013_review_intelligence_campaign_actions.sql"
SERVICE = "crates/homeserver-service/src/review_intelligence.rs"
AGENT = "crates/homeserver-service/src/agent_runtime.rs"
OPERATIONAL = "crates/homeserver-service/src/operational_data.rs"
TAURI = "src-tauri/src/review_intelligence.rs"
TAURI_AGENT = "src-tauri/src/agent.rs"
UI = "src/review-intelligence.js"
STYLE = "src/review-intelligence.css"

for table in (
    "review_intelligence_settings",
    "review_intelligence_runs",
    "review_observations",
    "review_clusters",
    "review_cluster_memberships",
    "review_recommendations",
    "review_recommendation_outcomes",
    "review_model_receipts",
    "provider_operational_sync_receipts",
    "provider_campaign_action_receipts",
):
    require(MIGRATION, table, f"review intelligence migration is missing {table}")

for marker in (
    '"/v1/review-intelligence"',
    '"/v1/review-intelligence/settings"',
    '"/v1/review-intelligence/sync"',
    '"/v1/review-intelligence/analyze"',
    '"/v1/review-intelligence/recommendations/outcome"',
    "deterministic_analysis",
    "sentiment_score",
    "build_clusters",
    "likely_causes",
    "suggested_fixes",
    "untrusted_provider_evidence",
    "provider_authoritative",
    "remote_context_allowed",
    '"store": false',
    "load_openai_key",
    "CREDENTIAL_SERVICE",
    "model_center::generate_text",
    "provider_post_json",
    '"/api/homeserver/operational-export.php"',
    '"/api/homeserver/campaign-actions.php"',
    "execute_campaign_plan",
    "campaign_execution_enabled",
    "run_automatic_processing_cycle",
    "automatic_processing_targets",
    "automatic_analysis_due",
    "AUTOMATIC_MAX_PAGES_PER_DATASET",
    "provider_campaign_action_receipts",
    'object.remove("merchant_approval_token")',
    'object.remove("merchant_approval_hash")',
    'object.remove("value_cents")',
):
    require(SERVICE, marker, f"review intelligence service boundary is missing {marker}")

for dataset in (
    "reviews.customer_reviews",
    "reviews.resolution_history",
    "conversations.threads",
    "conversations.messages",
    "conversations.follow_ups",
    "crm.contacts",
    "crm.activities",
    "crm.tasks",
    "crm.notes",
    "crm.consent",
    "commerce.orders",
    "commerce.order_items",
    "commerce.refunds",
    "gifts.ownership",
    "gifts.claims",
    "gifts.redemptions",
    "campaigns.definition",
    "campaigns.performance",
):
    require(OPERATIONAL, dataset, f"HomeServer operational catalog is missing {dataset}")

for action in (
    '"campaign.draft"',
    '"campaign.publish"',
    '"campaign.pause"',
    '"campaign.resume"',
    '"campaign.send_make_good"',
    '"campaign.send_authorized"',
):
    require(AGENT, action, f"supervised Agent Workspace is missing {action}")

require(AGENT, "review_intelligence::execute_campaign_plan", "campaign plans do not use the bounded review intelligence executor")
require(AGENT, "execute_approved_plan", "campaign execution is not inside the one-use approval engine")
require(AGENT, "approval.plan_hash == plan.plan_hash", "approval is not bound to the current plan hash")

for marker in (
    "homeserver_review_intelligence",
    "homeserver_update_review_intelligence_settings",
    "homeserver_sync_review_dataset",
    "homeserver_run_review_analysis",
    "homeserver_record_review_recommendation_outcome",
):
    require(TAURI, marker, f"Review Intelligence Tauri command is missing {marker}")
require(TAURI_AGENT, "homeserver_create_agent_plan", "supervised campaign plan creation command is missing")

for marker in (
    "Review Intelligence",
    "Deterministic core active",
    "The selected model is optional",
    "remote_context_allowed",
    "every 15 minutes",
    "Run Deterministic Analysis",
    "Prepare Supervised Campaign Plan",
    "review-plan-campaign-title",
    "leave blank to create a draft",
    "review-plan-reward-id",
    "A title is required to create a real Microgifter campaign draft",
    "A Microgifter campaign ID is required for publish, pause, resume, and send actions",
    "reward_template_id: rewardTemplateId || null",
    "homeserver_run_review_analysis",
    "homeserver_create_agent_plan",
):
    require(UI, marker, f"Review Intelligence UI is missing {marker}")

for marker in (
    ".review-intelligence-page",
    ".review-cluster-card",
    ".review-recommendation-card",
    ".review-settings-form",
):
    require(STYLE, marker, f"Review Intelligence style contract is missing {marker}")

require("crates/homeserver-service/src/main.rs", "mod review_intelligence;", "review intelligence service module is not registered")
require("crates/homeserver-service/src/app.rs", "review_intelligence::initialize(&connection)?", "review intelligence migration is not initialized")
require("crates/homeserver-service/src/app.rs", ".merge(review_intelligence::router(state.clone()))", "review intelligence router is not secured inside the local API")
require("crates/homeserver-service/src/app.rs", "run_review_intelligence_scheduler", "automatic Review Intelligence scheduler is not registered")
require("crates/homeserver-service/src/app.rs", "Duration::from_secs(15 * 60)", "automatic Review Intelligence scheduler is not bounded to a 15-minute cadence")
require("src-tauri/src/lib.rs", "mod review_intelligence;", "review intelligence Tauri module is not registered")
require("index.html", "/src/review-intelligence.js", "review intelligence frontend module is not loaded")

for path in (SERVICE, AGENT, UI):
    for forbidden in (
        "card_number",
        "cvv",
        "cvc",
        "payment_method_token",
        "shell.execute",
        "world_mission.dispatch",
    ):
        forbid(path, forbidden, f"{path} contains prohibited review intelligence capability or credential: {forbidden}")

for forbidden in (
    "agent_plan_approve",
    "agent_plan_execute_without_approval",
):
    forbid(SERVICE, forbidden, f"review intelligence may not self-approve through {forbidden}")

if ERRORS:
    print("Review intelligence validation failed:", file=sys.stderr)
    for error in ERRORS:
        print(f"- {error}", file=sys.stderr)
    raise SystemExit(1)

print("Deterministic review tracking, optional LLM analysis, provider evidence, and supervised campaign boundaries validated.")
