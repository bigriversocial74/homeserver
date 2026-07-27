from __future__ import annotations

import base64
from pathlib import Path

parts_directory = Path(__file__).resolve().parent / "phase4b_parts"
parts = sorted(parts_directory.glob("part*.b64"))
if len(parts) != 6:
    raise RuntimeError(f"expected 6 Phase 4B payload parts, found {len(parts)}")
source = b"".join(base64.b64decode(part.read_text(encoding="utf-8")) for part in parts)
exec(compile(source.decode("utf-8"), "phase4b_bootstrap_compiled.py", "exec"))
