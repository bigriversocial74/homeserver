#!/usr/bin/env python3
"""Validate the fixed, user-controlled Ollama installation assistant boundary."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ERRORS: list[str] = []


def source(path: str) -> str:
    file_path = ROOT / path
    if not file_path.is_file():
        ERRORS.append(f"required file is missing: {path}")
        return ""
    return file_path.read_text(encoding="utf-8")


def require(content: str, marker: str, message: str) -> None:
    if marker not in content:
        ERRORS.append(message)


def forbid(content: str, marker: str, message: str) -> None:
    if marker in content:
        ERRORS.append(message)


rust = source("src-tauri/src/ollama_install.rs")
frontend = source("src/ollama-install-assistant.js")
index = source("index.html")

for marker in (
    'const OLLAMA_WINDOWS_PAGE: &str = "https://ollama.com/download/windows"',
    'const OLLAMA_SETUP_URL: &str = "https://ollama.com/download/OllamaSetup.exe"',
    'const OLLAMA_INSTALL_COMMAND: &str = "irm https://ollama.com/install.ps1 | iex"',
    'match target.as_str()',
    '"installer" => launch_url(OLLAMA_SETUP_URL, "installer")',
    '"documentation" => launch_url(OLLAMA_WINDOWS_PAGE, "Windows documentation")',
    'Err("Unsupported Ollama setup target.".to_owned())',
    'Command::new("powershell.exe")',
    'Paste it here and press Enter when you are ready',
):
    require(rust, marker, f"native Ollama setup boundary is missing: {marker}")

for marker in (
    'invoke("homeserver_open_ollama_official", { target })',
    'invoke("homeserver_open_ollama_terminal")',
    'HomeServer displays and copies this command but never executes the remote script automatically.',
    'Invoke-RestMethod http://127.0.0.1:11434/api/version',
):
    require(frontend, marker, f"Ollama setup UI boundary is missing: {marker}")

require(
    index,
    '<script type="module" src="/src/ollama-install-assistant.js"></script>',
    "Ollama setup assistant is not loaded by the Control Center",
)

for marker in (
    '.arg(OLLAMA_INSTALL_COMMAND)',
    '.args([OLLAMA_INSTALL_COMMAND',
    'Command::new(OLLAMA_INSTALL_COMMAND)',
    'eval(',
    'new Function(',
):
    forbid(rust + frontend, marker, f"Ollama setup assistant contains disallowed execution path: {marker}")

if ERRORS:
    print("Ollama installation assistant validation failed:", file=sys.stderr)
    for error in ERRORS:
        print(f"- {error}", file=sys.stderr)
    raise SystemExit(1)

print("Ollama installation assistant validation passed.")
