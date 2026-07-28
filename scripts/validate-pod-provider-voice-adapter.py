#!/usr/bin/env python3
"""Validate the native POD provider voice-adapter security and compatibility boundaries."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ERRORS: list[str] = []


def read(path: str) -> str:
    target = ROOT / path
    if not target.is_file():
        ERRORS.append(f"required POD provider file is missing: {path}")
        return ""
    return target.read_text(encoding="utf-8")


def require(path: str, marker: str, message: str) -> None:
    if marker not in read(path):
        ERRORS.append(message)


def forbid(path: str, marker: str, message: str) -> None:
    if marker in read(path):
        ERRORS.append(message)


MIGRATION = "database/migrations/0015_pod_provider_voice_adapter.sql"
RUNTIME = "crates/homeserver-service/src/app/pod_provider_runtime.rs"
APP = "crates/homeserver-service/src/app.rs"
TAURI = "src-tauri/src/cloud.rs"
TAURI_LIB = "src-tauri/src/lib.rs"
UI = "src/cloud-connections.js"
CSS = "src/cloud-connections.css"
PACKAGE = "package.json"

for marker in (
    "CREATE TABLE IF NOT EXISTS pod_provider_connections",
    "CREATE TABLE IF NOT EXISTS pod_provider_runtime_profiles",
    "CREATE TABLE IF NOT EXISTS pod_provider_voice_jobs",
    "CREATE TABLE IF NOT EXISTS pod_provider_runtime_receipts",
    "CREATE TABLE IF NOT EXISTS pod_provider_worker_state",
    "FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE",
    "lease_credential_key TEXT NOT NULL UNIQUE",
    "UNIQUE (connection_id, remote_job_uuid)",
    "0015_pod_provider_voice_adapter",
):
    require(MIGRATION, marker, f"POD provider migration boundary is missing: {marker}")

for marker in (
    'const PROVIDER_KEY: &str = "pod"',
    'const CONTRACT_VERSION: &str = "pod-homeserver-voice-1"',
    'const CREDENTIAL_SERVICE: &str = "MicrogifterHomeServerPodConnections"',
    'const INSTALLATION_SERVICE: &str = "MicrogifterHomeServerPodIdentity"',
    'const LEASE_SERVICE: &str = "MicrogifterHomeServerPodJobLeases"',
    '"pod.pairing.v1"',
    '"pod.device-heartbeat.v1"',
    '"pod.voice.jobs.v1"',
    '"pod.voice.transcription.v1"',
    '"pod.voice.synthesis.v1"',
    '"pod.voice.artifacts.v1"',
    '"pod.voice.receipts.v1"',
    '"/v1/providers/pod/status"',
    '"/v1/providers/pod/connect"',
    '"/v1/providers/pod/runtime"',
    '"/v1/providers/pod/poll"',
    '"/v1/providers/pod/disconnect"',
    '"/api/homeserver/v1/pairing/exchange"',
    '"/api/homeserver/v1/devices/heartbeat"',
    '"/api/homeserver/v1/voice/jobs/poll"',
    '"/api/homeserver/v1/voice/jobs/complete"',
    '"/api/homeserver/v1/voice/jobs/fail"',
    '"/api/homeserver/v1/voice/artifacts/read"',
    "SigningKey::generate",
    "X-POD-Homeserver-ID",
    "X-POD-Connection-ID",
    "X-POD-Timestamp",
    "X-POD-Nonce",
    "X-POD-Signature",
    "sha256(body)",
    "reqwest::redirect::Policy::none()",
    'url.scheme() == "https"',
    "Command::new(path)",
    ".args(arguments)",
    "path.is_absolute() && path.is_file()",
    "kill_on_drop(true)",
    "maximum_input_bytes",
    "maximum_output_bytes",
    "payload_hash",
    "content_hash",
    "save_lease",
    "load_lease",
    "delete_lease",
    "save_secrets",
    "delete_secrets",
    "run_connection",
    "process_job",
    "receipt(",
    "maintain_history",
    "local_operation_available: true",
):
    require(RUNTIME, marker, f"POD provider runtime boundary is missing: {marker}")

for marker in (
    'pod_provider_runtime::initialize(&connection)',
    'pod_provider_runtime::run(state.clone(), shutdown.clone())',
    '.merge(pod_provider_runtime::router(state.clone()))',
    'pod_provider_runtime::maintain_history(&connection)',
    'pod_provider_worker.abort()',
):
    require(APP, marker, f"POD provider service integration is missing: {marker}")

for marker in (
    "homeserver_pod_status",
    "homeserver_connect_pod",
    "homeserver_update_pod_runtime",
    "homeserver_poll_pod",
    "homeserver_disconnect_pod",
    '"/v1/providers/pod/status"',
    '"/v1/providers/pod/connect"',
    '"/v1/providers/pod/runtime"',
    '"/v1/providers/pod/poll"',
    '"/v1/providers/pod/disconnect"',
):
    require(TAURI, marker, f"POD provider Tauri bridge is missing: {marker}")

for command in (
    "cloud::homeserver_pod_status",
    "cloud::homeserver_connect_pod",
    "cloud::homeserver_update_pod_runtime",
    "cloud::homeserver_poll_pod",
    "cloud::homeserver_disconnect_pod",
):
    require(TAURI_LIB, command, f"POD provider command is not registered: {command}")

for marker in (
    "Cloud & POD Connection Registry",
    "POD Wrapper",
    "POD Provider Connections",
    "Local voice runtime",
    "homeserver_pod_status",
    "homeserver_connect_pod",
    "homeserver_update_pod_runtime",
    "homeserver_poll_pod",
    "homeserver_disconnect_pod",
    "Browser voice remains the fallback",
    "Absolute executables only",
):
    require(UI, marker, f"POD provider Control Center boundary is missing: {marker}")

for marker in (
    ".pod-provider-section",
    ".pod-provider-card",
    ".pod-runtime-form",
    ".pod-runtime-grid",
    ".pod-runtime-health",
):
    require(CSS, marker, f"POD provider Control Center style is missing: {marker}")

# Existing Microgifter and generic connection commands must remain intact.
for marker in (
    "homeserver_pair_cloud_connection",
    "homeserver_sync_cloud_connection",
    "homeserver_disconnect_cloud_connection",
    "homeserver_microgifter_status",
    "homeserver_connect_microgifter",
    "homeserver_refresh_microgifter_entitlement",
    "homeserver_authorize_microgifter_update",
    "homeserver_complete_microgifter_device_replacement",
):
    require(TAURI, marker, f"existing provider command regressed: {marker}")
    require(TAURI_LIB, f"cloud::{marker}", f"existing provider command registration regressed: {marker}")

runtime = read(RUNTIME).split("#[cfg(test)]", 1)[0]
migration = read(MIGRATION)

# POD jobs must not become an authority path into private HomeServer systems.
for marker in (
    "knowledge_vault",
    "semantic_vault",
    "agent_runtime",
    "operational_data",
    "review_intelligence",
    "conversation",
    "prompt_text",
    "document_content",
    "authorize_update",
    "update_eligible=1",
    "signed_update",
):
    if marker in runtime.lower():
        ERRORS.append(f"POD provider runtime contains disallowed private or updater authority: {marker}")

# No raw secret material belongs in SQLite.
for marker in (
    "bearer_token TEXT",
    "device_private_key",
    "signing_seed TEXT",
    "lease_token TEXT",
    "sync_code TEXT",
    "audio_blob",
    "LONGBLOB",
):
    if marker.lower() in migration.lower():
        ERRORS.append(f"POD provider migration stores prohibited raw material: {marker}")

# Runtime commands are argv-only and must never invoke a shell.
for marker in (
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "/bin/sh",
    "sh -c",
    "Command::new(\"cmd\")",
    "Command::new(\"powershell\")",
    ".raw_arg(",
):
    if marker in runtime:
        ERRORS.append(f"POD provider runtime contains a disallowed shell boundary: {marker}")

# Remote connection addresses may come only from the paired POD base URL.
for marker in ("0.0.0.0", "Access-Control-Allow-Origin", "stun:", "turn:"):
    if marker in runtime:
        ERRORS.append(f"POD provider runtime contains a disallowed network boundary: {marker}")

require(PACKAGE, "validate-pod-provider-voice-adapter.py", "POD provider validation is not part of the permanent frontend gate")

if ERRORS:
    print("POD provider voice adapter validation failed:", file=sys.stderr)
    for error in ERRORS:
        print(f"- {error}", file=sys.stderr)
    raise SystemExit(1)

print("POD provider voice adapter boundaries validated.")
