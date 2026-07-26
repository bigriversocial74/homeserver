from __future__ import annotations

import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[1]


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    if content.count(old) != 1:
        raise RuntimeError(f"expected one match in {path}: {old!r}")
    target.write_text(content.replace(old, new, 1), encoding="utf-8")


replace_once(
    "crates/homeserver-service/src/knowledge_vault.rs",
    "use anyhow::{bail, ensure, Context, Result};",
    "use anyhow::{ensure, Context, Result};",
)
replace_once(
    "crates/homeserver-service/src/knowledge_vault.rs",
    "    fs::{self, File, OpenOptions},",
    "    fs::{self, OpenOptions},",
)
replace_once(
    "crates/homeserver-service/src/knowledge_vault.rs",
    "    let tags = serde_json::from_str(&tags_json).map_err(to_sql_error)?;",
    "    let tags = serde_json::from_str(&tags_json)\n        .map_err(|error| to_sql_error(error.into()))?;",
)

print("Phase 4A compile fixups applied.")
