#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "crates/homeserver-service/src/openrouter_provider.rs"
source = TARGET.read_text(encoding="utf-8")

replacements = [
    (
        "use rusqlite::{params, Connection, OptionalExtension};",
        "use rusqlite::{params, Connection};",
    ),
    (
        '''    ensure!(
        request.confirmation == "TEST REMOTE",
        "type TEST REMOTE to send this test prompt to OpenRouter"
    )
    .map_err(|error| action_error("openrouter_test_confirmation_required", error))?;''',
        '''    if request.confirmation != "TEST REMOTE" {
        return Err(action_error(
            "openrouter_test_confirmation_required",
            "type TEST REMOTE to send this test prompt to OpenRouter",
        ));
    }''',
    ),
    (
        '''    ensure!(
        request.confirmation == "DISCONNECT",
        "type DISCONNECT to remove the locally stored OpenRouter credential"
    )
    .map_err(|error| action_error("openrouter_disconnect_confirmation_required", error))?;''',
        '''    if request.confirmation != "DISCONNECT" {
        return Err(action_error(
            "openrouter_disconnect_confirmation_required",
            "type DISCONNECT to remove the locally stored OpenRouter credential",
        ));
    }''',
    ),
]

for old, new in replacements:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"expected one OpenRouter repair match, found {count}: {old[:90]!r}")
    source = source.replace(old, new, 1)

TARGET.write_text(source, encoding="utf-8")
print("OpenRouter handler type repairs applied")
