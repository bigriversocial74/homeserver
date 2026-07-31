#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HARDENING = ROOT / "scripts/phase21-hardening.py"


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, found {count}")
    return text.replace(old, new, 1)


# Make the main hardening helper tolerant of rustfmt's multiline output.
text = HARDENING.read_text(encoding="utf-8")
old_anchor = '''health_anchor = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(duplicate_members == 0, "evidence archive source membership is ambiguous");\n\n\'\'\'\nhealth_insert = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(duplicate_members == 0, "evidence archive source membership is ambiguous");\n\n'''
new_anchor = '''health_anchor = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(\n        duplicate_members == 0,\n        "evidence archive source membership is ambiguous"\n    );\n\n\'\'\'\nhealth_insert = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(\n        duplicate_members == 0,\n        "evidence archive source membership is ambiguous"\n    );\n\n'''
text = replace_once(text, old_anchor, new_anchor, "health anchor definition")

start_marker = "old_test = '''    fn evidence_allowlist_rejects_private_content_tables() {"
end_marker = "new_test = '''"
start = text.find(start_marker)
end = text.find(end_marker, start)
if start < 0 or end < 0:
    raise SystemExit("allowlist test anchor definition was not found")
formatted_test = '''old_test = \'\'\'    fn evidence_allowlist_rejects_private_content_tables() {\n        assert!(is_allowed_evidence_table("agent_runtime_receipts"));\n        assert!(is_allowed_evidence_table("model_inference_events"));\n        assert!(is_allowed_evidence_table(\n            "private_knowledge_egress_receipts"\n        ));\n        assert!(!is_allowed_evidence_table(\n            "model_inference_private_results"\n        ));\n        assert!(!is_allowed_evidence_table("agent_messages"));\n        assert!(!is_allowed_evidence_table("wrapper_job_payloads"));\n        assert!(!is_allowed_evidence_table("evidence_archive_events"));\n    }\n\'\'\'\n'''
text = text[:start] + formatted_test + text[end:]

# The two desktop exports share several source fragments. Remove the generic desktop
# patch section from the main helper; the exact evidence-export function is patched below.
desktop_start = text.find("# Desktop independently hashes every exported byte before recording an export receipt.")
desktop_end = text.find("# Native migration tests enforce the canonical seed hash and immutable chain identity.", desktop_start)
if desktop_start < 0 or desktop_end < 0:
    raise SystemExit("desktop hardening section was not found")
text = (
    text[:desktop_start]
    + "# Desktop evidence-export hashing was applied by phase21-hardening-anchor-fix.py.\n\n"
    + text[desktop_end:]
)
HARDENING.write_text(text, encoding="utf-8")

# Add the desktop hashing dependencies directly.
cargo_path = ROOT / "src-tauri/Cargo.toml"
cargo = cargo_path.read_text(encoding="utf-8")
if "sha2.workspace = true" not in cargo:
    cargo = replace_once(
        cargo,
        "serde_json.workspace = true\n",
        "serde_json.workspace = true\nsha2.workspace = true\nhex.workspace = true\n",
        "desktop hashing dependencies",
    )
cargo_path.write_text(cargo, encoding="utf-8")

# Patch only homeserver_export_evidence_archive, never recovery-package export.
lib_path = ROOT / "src-tauri/src/lib.rs"
lib = lib_path.read_text(encoding="utf-8")
if "use sha2::{Digest, Sha256};" not in lib:
    lib = replace_once(
        lib,
        "use serde::{de::DeserializeOwned, Deserialize, Serialize};\n",
        "use serde::{de::DeserializeOwned, Deserialize, Serialize};\nuse sha2::{Digest, Sha256};\n",
        "desktop SHA-256 import",
    )
function_start = lib.find("async fn homeserver_export_evidence_archive(")
function_end = lib.find("#[cfg_attr(mobile, tauri::mobile_entry_point)]", function_start)
if function_start < 0 or function_end < 0:
    raise SystemExit("evidence archive export function span was not found")
function = lib[function_start:function_end]
function = replace_once(
    function,
    '''    if package_sha256.len() != 64 || !package_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Evidence archive package hash is invalid.".to_owned());
    }
''',
    '''    if package_sha256.len() != 64 || !package_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Evidence archive package hash is invalid.".to_owned());
    }
    let expected_package_sha256 = package_sha256.to_ascii_lowercase();
''',
    "normalized expected export hash",
)
function = replace_once(
    function,
    '''    let mut stream = response.bytes_stream();
    let mut total_bytes = 0_u64;
    let transfer_result = async {
''',
    '''    let mut stream = response.bytes_stream();
    let mut total_bytes = 0_u64;
    let mut package_hasher = Sha256::new();
    let transfer_result = async {
''',
    "evidence export streaming hasher",
)
function = replace_once(
    function,
    '''            output
                .write_all(&chunk)
                .await
                .map_err(|error| error.to_string())?;
''',
    '''            package_hasher.update(&chunk);
            output
                .write_all(&chunk)
                .await
                .map_err(|error| error.to_string())?;
''',
    "hash evidence archive chunks",
)
function = replace_once(
    function,
    '''    if let Err(error) = transfer_result {
        drop(output);
        let _ = tokio::fs::remove_file(&destination_path).await;
        return Err(error);
    }

    let receipt: serde_json::Value = post_json(
''',
    '''    if let Err(error) = transfer_result {
        drop(output);
        let _ = tokio::fs::remove_file(&destination_path).await;
        return Err(error);
    }
    drop(output);
    let downloaded_package_sha256 = hex::encode(package_hasher.finalize());
    if downloaded_package_sha256 != expected_package_sha256 {
        let _ = tokio::fs::remove_file(&destination_path).await;
        return Err("Evidence archive export hash verification failed; the incomplete file was removed.".to_owned());
    }

    let receipt: serde_json::Value = post_json(
''',
    "verify evidence archive export digest",
)
function = replace_once(
    function,
    '            "package_sha256": package_sha256,\n',
    '            "package_sha256": expected_package_sha256,\n',
    "record verified evidence export digest",
)
lib = lib[:function_start] + function + lib[function_end:]
lib_path.write_text(lib, encoding="utf-8")

print("Phase 21 formatting anchors and evidence-only export hashing applied.")
