#!/usr/bin/env python3
from pathlib import Path

path = Path(__file__).resolve().parent / "validate-evidence-archive.py"
text = path.read_text(encoding="utf-8")
old = '''        '"model_inference_private_results"',
        '"documents"',
        '"credentials"',
'''
new = '''        '"model_inference_private_results"',
        '"future_private_events"',
        '"future_secret_receipts"',
'''
count = text.count(old)
if count != 1:
    raise SystemExit(f"closed allowlist validator anchor count was {count}")
path.write_text(text.replace(old, new, 1), encoding="utf-8")
print("Phase 21 closed allowlist validator expectations aligned.")
