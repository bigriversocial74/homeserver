from pathlib import Path

source_path = Path("scripts/phase3b_rollback_hardening.py")
source = source_path.read_text(encoding="utf-8")
marker = '\nworkflow = Path(".github/workflows/phase-1-foundation.yml")'
if marker not in source:
    raise SystemExit("workflow self-patch marker not found")
source = source.split(marker, 1)[0] + "\n"
exec(compile(source, str(source_path), "exec"), {"__name__": "__main__"})
