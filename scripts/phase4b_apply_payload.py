from __future__ import annotations

import hashlib
import tarfile
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
PARTS_DIR = ROOT / "scripts" / "phase4b_payload"
ARCHIVE_SHA256 = "74e3530d785a242a958ac2ba71fefed5d07ecf6ce18f6f05b4a7f02ead74e003"
EXPECTED_FILES = {
    "README.md",
    "Cargo.toml",
    "crates/homeserver-service/src/main.rs",
    "crates/homeserver-service/src/app.rs",
    "crates/homeserver-service/src/model_center.rs",
    "database/migrations/0006_model_center.sql",
    "docs/phase-4b-local-model-center.md",
    "scripts/smoke-test-service.ps1",
    "scripts/validate-security-boundaries.py",
    "src-tauri/src/lib.rs",
    "src-tauri/src/model.rs",
    "src/main.js",
    "src/styles.css",
}

parts = sorted(PARTS_DIR.glob("part*.bin"))
expected_names = [f"part{index:03d}.bin" for index in range(30)]
if [part.name for part in parts] != expected_names:
    raise SystemExit("Phase 4B payload parts are incomplete or out of order")

archive = b"".join(part.read_bytes() for part in parts)
actual_sha256 = hashlib.sha256(archive).hexdigest()
if actual_sha256 != ARCHIVE_SHA256:
    raise SystemExit(
        f"Phase 4B payload checksum mismatch: expected {ARCHIVE_SHA256}, got {actual_sha256}"
    )

archive_path = ROOT / ".phase4b-product.tar.gz"
archive_path.write_bytes(archive)
try:
    with tarfile.open(archive_path, "r:gz") as bundle:
        members = bundle.getmembers()
        names = {member.name for member in members}
        if names != EXPECTED_FILES:
            missing = sorted(EXPECTED_FILES - names)
            extra = sorted(names - EXPECTED_FILES)
            raise SystemExit(f"Unexpected Phase 4B payload contents; missing={missing}, extra={extra}")
        for member in members:
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or not member.isfile():
                raise SystemExit(f"Unsafe Phase 4B payload entry: {member.name}")
            source = bundle.extractfile(member)
            if source is None:
                raise SystemExit(f"Unable to read Phase 4B payload entry: {member.name}")
            destination = ROOT.joinpath(*path.parts)
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(source.read())
finally:
    archive_path.unlink(missing_ok=True)

model_center = ROOT / "crates" / "homeserver-service" / "src" / "model_center.rs"
source = model_center.read_text(encoding="utf-8")
replacements = {
    """fn read_settings(state: &AppState) -> Result<ModelSettings> {
    read_settings_from_connection(&state.connection()?)
}
""": """fn read_settings(state: &AppState) -> Result<ModelSettings> {
    let connection = state.connection()?;
    read_settings_from_connection(&connection)
}
""",
    """    statement
        .query_map([], operation_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}
""": """    let operations = statement
        .query_map([], operation_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)?;
    Ok(operations)
}
""",
}
for old, new in replacements.items():
    if source.count(old) != 1:
        raise SystemExit(f"Expected exactly one Phase 4B compiler repair marker: {old[:80]!r}")
    source = source.replace(old, new, 1)
model_center.write_text(source, encoding="utf-8")

print("Phase 4B product payload verified, repaired, and applied.")
