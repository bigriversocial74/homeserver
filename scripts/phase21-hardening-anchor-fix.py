#!/usr/bin/env python3
from pathlib import Path

path = Path(__file__).resolve().parent / "phase21-hardening.py"
text = path.read_text(encoding="utf-8")
old_anchor = '''health_anchor = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(duplicate_members == 0, "evidence archive source membership is ambiguous");\n\n\'\'\'\nhealth_insert = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(duplicate_members == 0, "evidence archive source membership is ambiguous");\n\n'''
new_anchor = '''health_anchor = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(\n        duplicate_members == 0,\n        "evidence archive source membership is ambiguous"\n    );\n\n\'\'\'\nhealth_insert = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(\n        duplicate_members == 0,\n        "evidence archive source membership is ambiguous"\n    );\n\n'''
if text.count(old_anchor) != 1:
    raise SystemExit(f"health anchor definition count was {text.count(old_anchor)}")
text = text.replace(old_anchor, new_anchor, 1)

start_marker = "old_test = '''    fn evidence_allowlist_rejects_private_content_tables() {"
end_marker = "new_test = '''"
start = text.find(start_marker)
end = text.find(end_marker, start)
if start < 0 or end < 0:
    raise SystemExit("allowlist test anchor definition was not found")
formatted_test = '''old_test = \'\'\'    fn evidence_allowlist_rejects_private_content_tables() {\n        assert!(is_allowed_evidence_table("agent_runtime_receipts"));\n        assert!(is_allowed_evidence_table("model_inference_events"));\n        assert!(is_allowed_evidence_table(\n            "private_knowledge_egress_receipts"\n        ));\n        assert!(!is_allowed_evidence_table(\n            "model_inference_private_results"\n        ));\n        assert!(!is_allowed_evidence_table("agent_messages"));\n        assert!(!is_allowed_evidence_table("wrapper_job_payloads"));\n        assert!(!is_allowed_evidence_table("evidence_archive_events"));\n    }\n\'\'\'\n'''
text = text[:start] + formatted_test + text[end:]
path.write_text(text, encoding="utf-8")
print("Phase 21 health and allowlist hardening anchors updated for rustfmt output.")
