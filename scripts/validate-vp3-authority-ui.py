from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
failures: list[str] = []


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def require(condition: bool, message: str) -> None:
    if not condition:
        failures.append(message)


index = read("index.html")
lib = read("src-tauri/src/lib.rs")
commands = read("src-tauri/src/vp3_authority.rs")
client = read("src/vp3-authority.js")
binding = read("crates/homeserver-service/src/vp3_device_binding.rs")
migration = read("database/migrations/0018_vp3_activation_update_client.sql")

require('/src/vp3-authority.js' in index, "Control Center does not load the optional VP3 provider UI module.")
require("mod vp3_authority;" in lib, "Tauri VP3 provider module is not registered.")
for command in (
    "homeserver_vp3_authority_status",
    "homeserver_vp3_device_identity",
    "homeserver_activate_vp3_authority",
    "homeserver_vp3_heartbeat",
    "homeserver_vp3_refresh_lease",
    "homeserver_vp3_check_update",
    "homeserver_vp3_download_update",
    "homeserver_vp3_submit_receipts",
    "homeserver_disconnect_vp3_authority",
):
    require(f"vp3_authority::{command}" in lib, f"Tauri command is not exposed: {command}")
    require(command in commands, f"Tauri optional VP3 adapter is missing: {command}")

for phrase in (
    "Optional paired provider",
    "VP3 Domain & POD Connection",
    "Microgifter remains the primary HomeServer account, pairing, entitlement, and signed-update authority",
    "Primary authority",
    "Connect Optional VP3",
    "Microgifter remains primary",
):
    require(phrase in client, f"Optional VP3 provider UI is missing required authority language: {phrase}")

for phrase in (
    "Software licensing and update authority",
    "VP3 HomeServer Activation",
    "Legacy fallback",
    "Activate VP3 Authority",
):
    require(phrase not in client, f"VP3 is still presented as the primary HomeServer authority: {phrase}")

require('"ACTIVATE VP3"' in client, "Optional VP3 connection confirmation is missing.")
require('"DISCONNECT VP3"' in client, "Optional VP3 disconnect confirmation is missing.")
require("homeserver_vp3_device_identity" in client, "Control Center does not load the local device identity.")
require("homeserver_activate_vp3_authority" in client, "Control Center cannot connect the optional VP3 provider.")
require("credential" in client and "enrollmentCode" in client, "One-time VP3 connection fields are missing.")
require("localStorage" not in client and "sessionStorage" not in client, "VP3 connection secrets are persisted in browser storage.")
require("MutationObserver" in client and "#vp3-authority-section" in client, "Optional VP3 controls do not remount after the main shell refreshes.")
require("deviceFingerprint" not in client and "device_fingerprint" not in commands, "VP3 connection adapter accepts a caller-controlled fingerprint.")
require("device-identity" in commands, "Tauri adapter cannot retrieve local device identity.")
require("bind_activation_identity" in binding, "Service VP3 identity-binding middleware is missing.")
require("MicrogifterHomeServer:vp3-device:" in binding, "Local VP3 fingerprint namespace changed.")
require("credential TEXT" not in migration and "enrollment_code TEXT" not in migration, "SQLite migration stores VP3 connection secrets.")

if failures:
    raise SystemExit("\n".join(failures))

print("Optional VP3 provider UI validation passed; Microgifter remains primary.")
