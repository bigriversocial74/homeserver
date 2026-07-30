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

require('/src/vp3-authority.js' in index, "Control Center does not load the VP3 authority UI module.")
require("mod vp3_authority;" in lib, "Tauri VP3 authority module is not registered.")
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
    require(command in commands, f"Tauri VP3 authority adapter is missing: {command}")

require('"ACTIVATE VP3"' in client, "Activation confirmation is missing.")
require('"DISCONNECT VP3"' in client, "Disconnect confirmation is missing.")
require("homeserver_vp3_device_identity" in client, "Control Center does not load the local device identity.")
require("homeserver_activate_vp3_authority" in client, "Control Center cannot activate VP3 authority.")
require("credential" in client and "enrollmentCode" in client, "One-time activation fields are missing.")
require("localStorage" not in client and "sessionStorage" not in client, "Activation secrets are persisted in browser storage.")
require("MutationObserver" in client and "#vp3-authority-section" in client, "VP3 controls do not remount after the main shell refreshes.")
require("deviceFingerprint" not in client and "device_fingerprint" not in commands, "Activation adapter accepts a caller-controlled fingerprint.")
require("device-identity" in commands, "Tauri adapter cannot retrieve local device identity.")
require("bind_activation_identity" in binding, "Service activation binding middleware is missing.")
require("MicrogifterHomeServer:vp3-device:" in binding, "Local fingerprint namespace changed.")
require("credential TEXT" not in migration and "enrollment_code TEXT" not in migration, "SQLite migration stores activation secrets.")

if failures:
    raise SystemExit("\n".join(failures))

print("Phase 14 VP3 authority UI validation passed.")
