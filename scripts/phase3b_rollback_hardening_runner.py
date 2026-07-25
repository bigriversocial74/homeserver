from pathlib import Path

source_path = Path("scripts/phase3b_rollback_hardening.py")
source = source_path.read_text(encoding="utf-8")
marker = '\nsmoke = Path("scripts/smoke-test-updater.ps1")'
if marker not in source:
    raise SystemExit("smoke patch marker not found")
source = source.split(marker, 1)[0] + "\n"
source = source.replace(
    '        None | Some(ref state) if state == "STOPPED" => return Ok(()),\n        Some(_) => {}',
    '        None => return Ok(()),\n        Some(state) if state == "STOPPED" => return Ok(()),\n        Some(_) => {}',
)
exec(compile(source, str(source_path), "exec"), {"__name__": "__main__"})
