from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_exact(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"Expected one {label} replacement in {path}, found {count}")
    path.write_text(text.replace(old, new), encoding="utf-8")


native = ROOT / "src-tauri/src/whisper.rs"
replace_exact(
    native,
    "params.set_progress_callback_safe(Some(move |progress| {\n",
    "params.set_progress_callback_safe(Some(move |progress: i32| {\n",
    "typed progress callback",
)

validator = ROOT / "scripts/validate-agent-whisper.py"
replace_exact(
    validator,
    """    ("set_progress_callback_safe", "bounded progress callback"),
""",
    """    ("set_progress_callback_safe", "bounded progress callback"),
    ("progress: i32", "explicit whisper-rs progress callback type"),
""",
    "progress type validator",
)
replace_exact(
    validator,
    """    ROOT / ".github/workflows/phase23c-callback-type-repair.yml",
):
""",
    """    ROOT / ".github/workflows/phase23c-callback-type-repair.yml",
    ROOT / "scripts/apply-phase23c-progress-type-repair.py",
    ROOT / ".github/workflows/phase23c-progress-type-repair.yml",
):
""",
    "progress repair cleanup denylist",
)

print("Applied the explicit whisper-rs progress callback i32 type.")
