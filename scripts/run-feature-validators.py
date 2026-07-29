#!/usr/bin/env python3
"""Run all fast feature validators and report every failure in one CI pass."""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = (
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

failures: list[str] = []
for script in SCRIPTS:
    path = ROOT / "scripts" / script
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
