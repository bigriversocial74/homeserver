from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

TEMPORARY_PHASE23_PATHS = (
    "scripts/apply-phase23-final-certification.py",
    "scripts/apply-phase23-linkage-fixture-repair.py",
    ".github/workflows/phase23-final-certification-repair.yml",
    ".github/workflows/phase23-linkage-fixture-repair.yml",
    ".github/workflows/phase23-test-fixture-fix.yml",
    ".github/workflows/phase23-cargo-lock-refresh.yml",
    ".github/workflows/phase23-lock-refresh.yml",
    ".github/workflows/phase-23-format-fix.yml",
)

remaining = [path for path in TEMPORARY_PHASE23_PATHS if (ROOT / path).exists()]
if remaining:
    raise SystemExit(
        "Temporary Phase 23 certification assets remain: " + ", ".join(remaining)
    )

print(
    "Phase 23A certification cleanliness validates that temporary repair, format, "
    "fixture, and dependency-lock workflows are absent from the permanent diff."
)
