#!/usr/bin/env python3
"""Apply the exact first-pass Rust compiler repairs for Phase 5B."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def patch(path: str, old: str, new: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one compiler-repair anchor, found {count}")
    target.write_text(content.replace(old, new, 1), encoding="utf-8", newline="\n")


patch(
    "crates/homeserver-service/src/app.rs",
    "mod cloud_registry;\n",
    "pub(crate) mod cloud_registry;\n",
)
patch(
    "crates/homeserver-service/src/agent_runtime.rs",
    "use crate::{cloud_registry, model_center, semantic_vault, AppState};",
    "use crate::{app::cloud_registry, model_center, semantic_vault, AppState};",
)
patch(
    "crates/homeserver-service/src/agent_runtime.rs",
    "    receipt_by_plan(&connection, &plan.plan_id)\n}",
    "    Ok(receipt_by_plan(&connection, &plan.plan_id)?)\n}",
)
patch(
    "crates/homeserver-service/src/agent_runtime.rs",
    '''            let title = object
                .get("title")
                .and_then(Value::as_str)
                .context("report title is required")?;
            let content = object
                .get("content_markdown")
                .and_then(Value::as_str)
                .context("report content is required")?;
            object.insert(
                "title".to_owned(),
                Value::String(sanitize_required_text(title, 180, "report title")?),
            );
            object.insert(
                "content_markdown".to_owned(),
                Value::String(sanitize_required_text(
                    content,
                    MAX_REPORT_CHARS,
                    "report content",
                )?),
            );''',
    '''            let title = sanitize_required_text(
                object
                    .get("title")
                    .and_then(Value::as_str)
                    .context("report title is required")?,
                180,
                "report title",
            )?;
            let content = sanitize_required_text(
                object
                    .get("content_markdown")
                    .and_then(Value::as_str)
                    .context("report content is required")?,
                MAX_REPORT_CHARS,
                "report content",
            )?;
            object.insert("title".to_owned(), Value::String(title));
            object.insert("content_markdown".to_owned(), Value::String(content));''',
)
print("Focused Agent Workspace compiler repairs applied.")
