from __future__ import annotations

import re
import sqlite3
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FAILURES: list[str] = []


def read(path: str) -> str:
    target = ROOT / path
    if not target.is_file():
        FAILURES.append(f"missing {path}")
        return ""
    return target.read_text(encoding="utf-8")


def require(condition: bool, message: str) -> None:
    if not condition:
        FAILURES.append(message)


migration = read("database/migrations/0019_federated_settings.sql")
service = read("crates/homeserver-service/src/federated_settings.rs")
signature = read("crates/homeserver-service/src/federated_settings_signature.rs")
app = read("crates/homeserver-service/src/app.rs")
tauri = read("src-tauri/src/lib.rs")
tauri_commands = read("src-tauri/src/federated_settings.rs")
frontend = read("src/federated-settings.js")
index = read("index.html")

require("0019_federated_settings" in migration, "migration key is missing")
require("CHECK (authority IN ('vp3','homeserver','shared'))" in migration, "authority constraint is missing")
require("sensitivity='non_secret'" in migration, "non-secret catalog boundary is missing")
require("federated_settings_sync_receipts" in migration, "sync receipt table is missing")
require("dirty INTEGER" in migration and "cloud_revision INTEGER" in migration, "local dirty/cloud revision state is missing")
require("INSERT OR REPLACE INTO federated_setting_catalog" not in migration, "catalog refresh uses destructive SQLite REPLACE")
require("ON CONFLICT(setting_key) DO UPDATE SET" in migration, "catalog refresh is not restart-safe")

catalog_keys = set(re.findall(r"\('([a-z][a-z0-9_.-]+)'\s*,", migration))
expected_keys = {
    "appearance.theme",
    "regional.locale",
    "regional.timezone",
    "updates.channel",
    "updates.auto_download",
    "updates.install_window",
    "notifications.email_enabled",
    "notifications.desktop_enabled",
    "privacy.telemetry_level",
    "commerce.default_currency",
    "commerce.receipt_email_enabled",
}
require(expected_keys <= catalog_keys, "local federated settings catalog is incomplete")
for forbidden in ("secret", "password", "credential", "private_key", "api_key", "token", "prompt", "conversation"):
    require(not any(forbidden in key for key in catalog_keys), f"secret-like catalog key contains {forbidden}")

# Execute the migration twice with foreign keys enabled and prove a saved local value survives
# the catalog seed refresh. SQLite REPLACE would delete the parent row and cascade this value.
try:
    connection = sqlite3.connect(":memory:")
    connection.execute("PRAGMA foreign_keys=ON")
    connection.execute("CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY)")
    connection.executescript(migration)
    connection.execute(
        "INSERT INTO federated_setting_values "
        "(setting_key,value_json,value_hash,local_revision,cloud_revision,source_authority,dirty,updated_at_utc) "
        "VALUES (?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        ("appearance.theme", '"dark"', "a" * 64, 1, 0, "homeserver", 1),
    )
    connection.executescript(migration)
    retained = connection.execute(
        "SELECT value_json,local_revision,dirty FROM federated_setting_values WHERE setting_key='appearance.theme'"
    ).fetchone()
    require(retained == ('"dark"', 1, 1), "catalog refresh deleted or rewrote the saved local setting")
    migration_count = connection.execute(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key='0019_federated_settings'"
    ).fetchone()[0]
    require(migration_count == 1, "repeat migration did not retain one migration registration")
finally:
    try:
        connection.close()
    except NameError:
        pass

require("this setting is controlled by VP3" in service, "local writes do not reject VP3 authority")
require("expected_local_revision" in service, "local optimistic revision check is missing")
require("preserve_local" in service and "last_conflict_reason" in service, "conflict-safe local preservation is missing")
require("bearer_auth(credential)" in service, "VP3 sync does not use the OS-vault device credential")
require("Policy::none" in service or "redirect(reqwest::redirect::Policy::none())" in service, "settings sync does not reject redirects")
require("validate_cloud_snapshot(state" in service, "signed cloud snapshot validation is not called")
require("SignedSnapshotEvidence" in service, "signed snapshot evidence is not bound to the merge")
for merge_field in ("generated_at", "replayed", "applied", "conflicts"):
    require(f"{merge_field}:" in service, f"signed snapshot evidence does not bind {merge_field}")
    require(f'"{merge_field}":' in signature, f"signed document equality does not bind {merge_field}")
require("Ed25519" in signature and "snapshot has expired" in signature, "Ed25519 and expiration verification are incomplete")
require("wrapper does not match its signed document" in signature, "signed document/wrapper equality is not enforced")
require("signature verification failed" in signature, "signature failure path is missing")
require("sync_tampering_is_rejected" in signature, "signed merge-instruction tamper test is missing")

require(app.count('mod federated_settings;') == 1, "federated settings service module is not wired exactly once")
require(app.count('mod federated_settings_signature;') == 1, "signature module is not wired exactly once")
require(app.count("federated_settings::initialize(&connection)?;") == 1, "federated settings migration is not initialized exactly once")
require(app.count("merge(federated_settings::router(state.clone()))") == 1, "federated settings router is not merged exactly once")
require(tauri.count("mod federated_settings;") == 1, "Tauri federated settings module is not wired exactly once")
for command in (
    "homeserver_federated_settings",
    "homeserver_update_federated_setting",
    "homeserver_sync_federated_settings",
):
    require(command in tauri and command in tauri_commands, f"missing Tauri command {command}")
require(index.count('/src/federated-settings.js') == 1, "federated settings frontend is not loaded exactly once")
require("localStorage" not in frontend and "sessionStorage" not in frontend, "federated settings use browser persistence")
require("VP3 authority" in frontend and "HomeServer authority" in frontend and "Shared authority" in frontend, "authority terminology is inconsistent")
require("Privacy boundary" in frontend, "privacy boundary is missing from Control Center")

if FAILURES:
    print("Phase 15 federated settings validation failed:", file=sys.stderr)
    for failure in FAILURES:
        print(f"- {failure}", file=sys.stderr)
    raise SystemExit(1)

print("Phase 15 federated settings validation passed.")
