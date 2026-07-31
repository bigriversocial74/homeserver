#!/usr/bin/env python3
from pathlib import Path

path = Path(__file__).resolve().parents[1] / "crates/homeserver-service/src/evidence_archive.rs"
text = path.read_text(encoding="utf-8")

replacements = [
    ("    let mut connection = state.connection()?;", "    let connection = state.connection()?;", "connection mutability"),
    ("        let mut entry = entry?;", "        let entry = entry?;", "tar entry mutability"),
    ("first_record_at.as_ref().is_none_or(|current| value < current)", "first_record_at.as_ref().map_or(true, |current| value < current)", "minimum timestamp MSRV"),
    ("last_record_at.as_ref().is_none_or(|current| value > current)", "last_record_at.as_ref().map_or(true, |current| value > current)", "maximum timestamp MSRV"),
    ("fn build_package(\n", "#[allow(clippy::too_many_arguments)]\nfn build_package(\n", "package boundary lint"),
    ("fn record_event_tx(\n", "#[allow(clippy::too_many_arguments)]\nfn record_event_tx(\n", "event boundary lint"),
]
for old, new, label in replacements:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, found {count}")
    text = text.replace(old, new, 1)
path.write_text(text, encoding="utf-8")
print("Phase 21 strict lint and Rust 1.80 compatibility repairs applied.")
