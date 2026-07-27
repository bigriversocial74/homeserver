#!/usr/bin/env python3
"""Correct the one ambiguous call-tool insertion anchor in the temporary integration patch."""
from pathlib import Path

path = Path(__file__).resolve().with_name("apply-phase5b-agent-workspace.py")
lines = path.read_text(encoding="utf-8").splitlines()
replacement = [
    "if anchor not in content:",
    "    raise SystemExit(f\"{path}: MCP call-tool insertion anchor is missing\")",
    "content = content.replace(anchor, '            (payload, \\\"knowledge.read\\\")\\n        }\\n' + call_arms + '        _ => {', 1)",
]
matched = False
updated: list[str] = []
for line in lines:
    if line.startswith("content = replace_once(content, anchor,"):
        updated.extend(replacement)
        matched = True
    else:
        updated.append(line)
if not matched:
    raise SystemExit("temporary integration patch anchor statement was not found")
path.write_text("\n".join(updated) + "\n", encoding="utf-8", newline="\n")
print("Temporary MCP insertion anchor corrected.")
