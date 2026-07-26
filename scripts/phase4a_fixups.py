from __future__ import annotations

import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    if content.count(old) != 1:
        raise RuntimeError(f"expected one match in {path}: {old!r}")
    target.write_text(content.replace(old, new, 1), encoding="utf-8")


def replace_regex(path: str, pattern: str, replacement: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    updated, count = re.subn(pattern, lambda _match: replacement, content, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"expected one regex match in {path}: {pattern!r}")
    target.write_text(updated, encoding="utf-8")


SERVICE = "crates/homeserver-service/src/knowledge_vault.rs"

replace_once(
    SERVICE,
    "use anyhow::{bail, ensure, Context, Result};",
    "use anyhow::{ensure, Context, Result};",
)
replace_once(
    SERVICE,
    "    body::Body,",
    "    body::Bytes,",
)
replace_once(
    SERVICE,
    "use futures_util::StreamExt;\n",
    "",
)
replace_once(
    SERVICE,
    "    fs::{self, File, OpenOptions},",
    "    fs::{self, OpenOptions},",
)
replace_once(
    SERVICE,
    "    let tags = serde_json::from_str(&tags_json).map_err(to_sql_error)?;",
    "    let tags = serde_json::from_str(&tags_json)\n        .map_err(|error| to_sql_error(error.into()))?;",
)
replace_regex(
    SERVICE,
    r'''    body: Body,
\) -> ApiResult<VaultActionResult> \{
    let file_name = decode_header\(&headers, FILE_NAME_HEADER, 1024, "vault_file_name_invalid"\)\?;
    let tags_json = decode_header\(&headers, TAGS_HEADER, 8192, "vault_tags_invalid"\)\?;
    let tags: Vec<String> = serde_json::from_str\(&tags_json\)
        \.map_err\(\|error\| action_error\("vault_tags_invalid", error\.into\(\)\)\)\?;
    let mut stream = body\.into_data_stream\(\);
    let mut bytes = Vec::new\(\);
    while let Some\(chunk\) = stream\.next\(\)\.await \{
.*?    \}
    if bytes\.is_empty\(\) \{''',
    '''    body: Bytes,
) -> ApiResult<VaultActionResult> {
    let file_name = decode_header(&headers, FILE_NAME_HEADER, 1024, "vault_file_name_invalid")?;
    let tags_json = decode_header(&headers, TAGS_HEADER, 8192, "vault_tags_invalid")?;
    let tags: Vec<String> = serde_json::from_str(&tags_json)
        .map_err(|error| action_error("vault_tags_invalid", error.into()))?;
    let bytes = body.to_vec();
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(action_error(
            "vault_import_too_large",
            anyhow::anyhow!("document exceeds the 16 MB import limit"),
        ));
    }
    if bytes.is_empty() {''',
)
replace_once(
    SERVICE,
    "    text.match_indices(&query).count().min(u32::MAX as usize) as u32",
    "    text.match_indices(query.as_str())\n        .count()\n        .min(u32::MAX as usize) as u32",
)
replace_once(
    SERVICE,
    "    let byte_position = lower.find(&query).unwrap_or(0);",
    "    let byte_position = lower.find(query.as_str()).unwrap_or(0);",
)
replace_once(
    SERVICE,
    "SELECT COUNT(*),SUM(CASE WHEN state='indexed' THEN 1 ELSE 0 END),SUM(CASE WHEN state='changed' THEN 1 ELSE 0 END),SUM(CASE WHEN state='missing' THEN 1 ELSE 0 END),SUM(CASE WHEN state='failed' THEN 1 ELSE 0 END),COALESCE(SUM(size_bytes),0),MAX(indexed_at_utc) FROM vault_documents",
    "SELECT COUNT(*),COALESCE(SUM(CASE WHEN state='indexed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN state='changed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN state='missing' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN state='failed' THEN 1 ELSE 0 END),0),COALESCE(SUM(size_bytes),0),MAX(indexed_at_utc) FROM vault_documents",
)

print("Phase 4A compile and empty-state fixups applied.")
