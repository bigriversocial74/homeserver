#!/usr/bin/env python3
"""Validate operational evidence, provenance, and supervised action boundaries."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ERRORS: list[str] = []


def read(path: str) -> str:
    target = ROOT / path
    if not target.is_file():
        ERRORS.append(f"required operational data file is missing: {path}")
        return ""
    return target.read_text(encoding="utf-8")


def require(path: str, marker: str, message: str) -> None:
    if marker not in read(path):
        ERRORS.append(message)


def forbid(path: str, marker: str, message: str) -> None:
    if marker in read(path):
        ERRORS.append(message)


MIGRATION = "database/migrations/0012_operational_data_import.sql"
SERVICE = "crates/homeserver-service/src/operational_data.rs"
AGENT = "crates/homeserver-service/src/agent_runtime.rs"
REVIEW = "crates/homeserver-service/src/review_intelligence.rs"
TAURI = "src-tauri/src/operational.rs"
UI = "src/operational-data.js"
STYLE = "src/operational-data.css"
DOC = "docs/phase-5c-operational-data-import.md"

for table in (
    "operational_provider_manifests",
    "operational_dataset_catalog",
    "operational_dataset_grants",
    "operational_import_runs",
    "operational_import_cursors",
    "operational_raw_records",
    "operational_entities",
    "operational_entity_versions",
    "operational_events",
    "operational_provenance",
    "operational_retention_policies",
    "operational_import_errors",
):
    require(MIGRATION, table, f"operational data migration is missing {table}")

for marker in (
    '"/v1/operational-data"',
    '"/v1/operational-data/grants"',
    '"/v1/operational-data/import"',
    '"/v1/operational-data/query"',
    "MAX_RECORDS_PER_IMPORT",
    "MAX_EVENTS_PER_IMPORT",
    "untrusted_provider_evidence",
    "provider_authoritative",
    "provider dataset is not declared",
    "dataset is not authorized for import",
    "scope does not match the paired connection",
    "operational_provenance",
    "cursor_after",
    "source_revision",
    "payload_hash",
    "query_for_agent",
    "import_for_provider",
):
    require(SERVICE, marker, f"operational data service boundary is missing {marker}")

for dataset in (
    "merchant.profile",
    "merchant.locations",
    "merchant.products",
    "merchant.inventory",
    "merchant.staff",
    "merchant.store_activity",
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
    "campaigns.authorizations",
    "creator.attribution",
):
    require(SERVICE, dataset, f"Microgifter operational manifest is missing {dataset}")

for marker in (
    "sentiment_analysis",
    "semantic_clustering",
    "conversation_continuity",
    "service_recovery",
    "campaign_management",
    "consent_enforcement",
):
    require(SERVICE, marker, f"expanded operational agent-use catalog is missing {marker}")

for marker in (
    "homeserver_operational_data",
    "homeserver_update_operational_dataset_grant",
    "homeserver_import_operational_data",
    "homeserver_query_operational_data",
):
    require(TAURI, marker, f"Control Center command is missing {marker}")

for marker in (
    "Operational Data",
    "Provider authority preserved",
    "Explicit dataset grants",
    "Untrusted evidence boundary",
    "homeserver_update_operational_dataset_grant",
    "homeserver_query_operational_data",
    "Reviews, messages, CRM contact details, purchase history, and gift ownership",
):
    require(UI, marker, f"Operational Data UI is missing {marker}")

for marker in (
    ".operational-page",
    ".operational-dataset-card",
    ".operational-evidence",
    ".operational-modal-backdrop",
):
    require(STYLE, marker, f"Operational Data style contract is missing {marker}")

for marker in (
    "Connected platforms remain authoritative",
    "untrusted_provider_evidence",
    "Snapshot",
    "Incremental",
    "Event",
    "World Agent dispatch",
):
    require(DOC, marker, f"Operational Data documentation is missing {marker}")

require("crates/homeserver-service/src/main.rs", "mod operational_data;", "operational data service module is not registered")
require("crates/homeserver-service/src/app.rs", "operational_data::initialize(&connection)?", "operational data migration is not initialized")
require("crates/homeserver-service/src/app.rs", ".merge(operational_data::router(state.clone()))", "operational data router is not inside the secured local API")
require("src-tauri/src/lib.rs", "mod operational;", "operational data Tauri module is not registered")
require("index.html", "/src/operational-data.js", "operational data frontend module is not loaded")
require("package.json", "validate-operational-data.py", "operational data validator is not part of frontend validation")
require(AGENT, "dataset:", "Agent Workspace does not support connection-bound operational datasets")
require(AGENT, "operational_data::query_for_agent", "Agent Workspace does not query authorized operational evidence")

# Operational evidence cannot execute anything by itself. Campaign actions are
# allowed only in the supervised Agent Workspace allowlist and must delegate to
# the Review Intelligence provider adapter after one-use local approval.
for path in (SERVICE, UI, DOC):
    for forbidden in (
        "campaign.publish",
        "campaign.send_make_good",
        "campaign.send_authorized",
        "reward.issue",
        "claim.redeem",
        "payment.execute",
        "shell.execute",
        "world_mission.dispatch",
    ):
        forbid(path, forbidden, f"{path} contains a direct operational capability: {forbidden}")

for marker in (
    '"campaign.draft"',
    '"campaign.publish"',
    '"campaign.pause"',
    '"campaign.resume"',
    '"campaign.send_make_good"',
    '"campaign.send_authorized"',
    "review_intelligence::execute_campaign_plan",
):
    require(AGENT, marker, f"supervised campaign action boundary is missing {marker}")

for marker in (
    "campaign_execution_enabled",
    "provider_post_json",
    '"/api/homeserver/campaign-actions.php"',
    "provider_campaign_action_receipts",
):
    require(REVIEW, marker, f"provider campaign execution boundary is missing {marker}")

# The product UI may manage grants and query evidence, but must not expose a
# free-form import editor or a direct provider campaign-send call.
forbid(UI, "homeserver_import_operational_data", "Control Center exposes free-form operational import")
forbid(UI, "campaign-actions.php", "Operational Data UI calls the provider campaign endpoint directly")

if ERRORS:
    print("Operational data validation failed:", file=sys.stderr)
    for error in ERRORS:
        print(f"- {error}", file=sys.stderr)
    raise SystemExit(1)

print("Operational manifests, sensitive dataset grants, provenance, evidence, and supervised campaign boundaries validated.")
