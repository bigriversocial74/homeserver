#!/usr/bin/env python3
from pathlib import Path

path = Path(__file__).resolve().parent / "phase21-hardening.py"
text = path.read_text(encoding="utf-8")
old_anchor = '''health_anchor = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(duplicate_members == 0, "evidence archive source membership is ambiguous");\n\n\'\'\'\nhealth_insert = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(duplicate_members == 0, "evidence archive source membership is ambiguous");\n\n'''
new_anchor = '''health_anchor = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(\n        duplicate_members == 0,\n        "evidence archive source membership is ambiguous"\n    );\n\n\'\'\'\nhealth_insert = \'\'\'    let duplicate_members: i64 = connection.query_row(\n        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",\n        [],\n        |row| row.get(0),\n    )?;\n    ensure!(\n        duplicate_members == 0,\n        "evidence archive source membership is ambiguous"\n    );\n\n'''
if text.count(old_anchor) != 1:
    raise SystemExit(f"health anchor definition count was {text.count(old_anchor)}")
path.write_text(text.replace(old_anchor, new_anchor, 1), encoding="utf-8")
print("Phase 21 health hardening anchor updated for rustfmt output.")
