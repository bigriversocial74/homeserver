#!/usr/bin/env python3
"""One-time deterministic source repair for Phase 5B strict Clippy findings."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
connector_path = ROOT / "crates/homeserver-service/src/cloud_connector.rs"
registry_path = ROOT / "crates/homeserver-service/src/cloud_registry.rs"

connector = connector_path.read_text(encoding="utf-8")
connector = connector.replace("use tokio::sync::watch;\n", "")
connector = connector.replace("use tracing::{info, warn};", "use tracing::warn;")
connector = connector.replace("const SYNC_INTERVAL: Duration = Duration::from_secs(60);\n", "")
start_marker = "\npub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {\n"
end_marker = "\nimpl AppState {"
if start_marker in connector:
    start = connector.index(start_marker)
    end = connector.index(end_marker, start)
    connector = connector[:start] + connector[end:]
elif "pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>)" in connector:
    raise SystemExit("Unable to isolate the superseded singleton sync worker")
connector_path.write_text(connector, encoding="utf-8", newline="\n")

registry = registry_path.read_text(encoding="utf-8")
legacy = ".is_none_or(|value| value.len() <= 120)"
compatible = ".map_or(true, |value| value.len() <= 120)"
if legacy in registry:
    registry = registry.replace(legacy, compatible)
elif compatible not in registry:
    raise SystemExit("Expected receipt reason validation expression was not found")
registry_path.write_text(registry, encoding="utf-8", newline="\n")
