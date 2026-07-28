#!/usr/bin/env python3
from pathlib import Path

path = Path(__file__).resolve().parent / "apply-phase5c-review-intelligence.py"
value = path.read_text(encoding="utf-8")
helper_anchor = '''def replace_once(value: str, old: str, new: str, label: str) -> str:
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, found {count}")
    return value.replace(old, new, 1)
'''
helper_replacement = helper_anchor + '''\n\ndef replace_first(value: str, old: str, new: str, label: str) -> str:
    if old not in value:
        raise SystemExit(f"{label}: anchor was not found")
    return value.replace(old, new, 1)
'''
if "def replace_first" not in value:
    if helper_anchor not in value:
        raise SystemExit("replace helper anchor was not found")
    value = value.replace(helper_anchor, helper_replacement, 1)
old = '''agent = replace_once(
    agent,
    """        "report.save" => {
'''
new = '''agent = replace_first(
    agent,
    """        "report.save" => {
'''
if old in value:
    value = value.replace(old, new, 1)
elif new not in value:
    raise SystemExit("campaign executor patch anchor was not found")
path.write_text(value, encoding="utf-8", newline="\n")
print("Review intelligence patcher restricted to first production executor occurrence.")
