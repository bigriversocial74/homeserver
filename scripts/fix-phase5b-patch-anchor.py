#!/usr/bin/env python3
"""Correct the one ambiguous call-tool insertion anchor in the temporary integration patch."""
from pathlib import Path

path = Path(__file__).resolve().with_name("apply-phase5b-agent-workspace.py")
content = path.read_text(encoding="utf-8")
old = "content = replace_once(content, anchor, '            (payload, \\\"knowledge.read\\\")\\n        }\\n' + call_arms + '        _ => {', path)"
new = "if anchor not in content:\n    raise SystemExit(f\"{path}: MCP call-tool insertion anchor is missing\")\ncontent = content.replace(anchor, '            (payload, \\\"knowledge.read\\\")\\n        }\\n' + call_arms + '        _ => {', 1)"
if old not in content:
    raise SystemExit("temporary integration patch anchor statement was not found")
path.write_text(content.replace(old, new, 1), encoding="utf-8", newline="\n")
print("Temporary MCP insertion anchor corrected.")
