#!/usr/bin/env python3
"""Apply exact behavioral-test fixture and unused-parameter repairs."""
from pathlib import Path

path = Path(__file__).resolve().parents[1] / "crates/homeserver-service/src/agent_runtime.rs"
content = path.read_text(encoding="utf-8")
replacements = [
    (
        "    let assistant_text = generate_grounded_response(\n        state.clone(),\n",
        "    let assistant_text = generate_grounded_response(\n",
    ),
    (
        "async fn generate_grounded_response(\n    state: Arc<AppState>,\n",
        "async fn generate_grounded_response(\n",
    ),
    (
        "        let connection = database::initialize(&directory.path().join(\"agent-runtime.sqlite3\")).unwrap();\n        initialize(&connection).unwrap();\n",
        "        let connection = database::initialize(&directory.path().join(\"agent-runtime.sqlite3\")).unwrap();\n        cloud_registry::initialize(&connection).unwrap();\n        initialize(&connection).unwrap();\n",
    ),
]
for old, new in replacements:
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"expected one Agent Workspace test-repair anchor, found {count}: {old[:90]!r}")
    content = content.replace(old, new, 1)
path.write_text(content, encoding="utf-8", newline="\n")
print("Agent Workspace behavioral test fixture repaired.")
