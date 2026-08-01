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
    "use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};\n",
    "use whisper_rs::{\n    FullParams, SamplingStrategy, SegmentCallbackData, WhisperContext,\n    WhisperContextParameters,\n};\n",
    "SegmentCallbackData import",
)
replace_exact(
    native,
    "params.set_segment_callback_safe_lossy(Some(move |data| {\n",
    "params.set_segment_callback_safe_lossy(Some(move |data: SegmentCallbackData| {\n",
    "typed segment callback",
)

validator = ROOT / "scripts/validate-agent-whisper.py"
replace_exact(
    validator,
    """    ("set_segment_callback_safe_lossy", "partial transcript callback"),
""",
    """    ("set_segment_callback_safe_lossy", "partial transcript callback"),
    ("data: SegmentCallbackData", "explicit whisper-rs callback type"),
""",
    "callback type validator",
)
replace_exact(
    validator,
    """    ROOT / ".github/workflows/phase23c-removal-hardening.yml",
):
""",
    """    ROOT / ".github/workflows/phase23c-removal-hardening.yml",
    ROOT / "scripts/apply-phase23c-callback-type-repair.py",
    ROOT / ".github/workflows/phase23c-callback-type-repair.yml",
):
""",
    "callback repair cleanup denylist",
)

print("Applied the explicit whisper-rs SegmentCallbackData callback type.")
