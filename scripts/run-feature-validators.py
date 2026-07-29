#!/usr/bin/env python3
"""Run all fast feature validators and report every failure in one CI pass."""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SCRIPTS = (
    "validate-validation-support.py",
    "validate-ollama-install-assistant.py",
    "validate-document-extraction.py",
    "validate-mcp-runtime.py",
    "validate-agent-workspace.py",
    "validate-agent-chat-route.py",
    "validate-notification-menu.py",
    "validate-pairing-onboarding.py",
    "validate-operational-data.py",
    "validate-review-intelligence.py",
    "validate-multi-cloud-connections.py",
    "validate-windows-desktop.py",
)

requested = tuple(sys.argv[1:])
SCRIPTS = requested or DEFAULT_SCRIPTS

invalid = [
    script
    for script in SCRIPTS
    if not script.startswith("validate-")
    or not script.endswith(".py")
    or "/" in script
    or "\\" in script
    or script in ("validate-security-boundaries.py",)
]
if invalid:
    print(
        "HomeServer feature validator registration contains invalid entries: "
        + ", ".join(invalid),
        file=sys.stderr,
    )
    raise SystemExit(1)

if len(set(SCRIPTS)) != len(SCRIPTS):
    print("HomeServer feature validator registration contains duplicates.", file=sys.stderr)
    raise SystemExit(1)

failures: list[str] = []
for script in SCRIPTS:
    path = ROOT / "scripts" / script
    if not path.is_file():
        print(f"HomeServer feature validator is missing: {script}", file=sys.stderr)
        failures.append(script)
        continue

    result = subprocess.run(
        [sys.executable, str(path)],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.stdout:
        print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    if result.returncode != 0:
        failures.append(script)

if failures:
    print(
        "HomeServer feature validation failed in: " + ", ".join(failures),
        file=sys.stderr,
    )
    raise SystemExit(1)

print("All HomeServer feature validators passed.")
