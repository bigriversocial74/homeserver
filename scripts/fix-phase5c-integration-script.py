#!/usr/bin/env python3
"""Repair raw-string escaping in the temporary Phase 5C integration patcher."""
from pathlib import Path

path = Path(__file__).resolve().parents[1] / "scripts/apply-phase5c-operational-data.py"
value = path.read_text(encoding="utf-8")
old_anchor = """agent = replace_once(
    agent,
    '''    Ok(format!(
"""
new_anchor = """agent = replace_once(
    agent,
    r'''    Ok(format!(
"""
if value.count(old_anchor) != 1:
    raise SystemExit(f"fallback old raw-string anchor count was {value.count(old_anchor)}")
value = value.replace(old_anchor, new_anchor, 1)
old_replacement = """    '''    let evidence_lines = operational
"""
new_replacement = """    r'''    let evidence_lines = operational
"""
if value.count(old_replacement) != 1:
    raise SystemExit(f"fallback replacement raw-string anchor count was {value.count(old_replacement)}")
value = value.replace(old_replacement, new_replacement, 1)
path.write_text(value, encoding="utf-8", newline="\n")
print("Phase 5C integration patcher raw-string escaping repaired.")
