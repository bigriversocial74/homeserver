from pathlib import Path

path = Path("crates/homeserver-service/src/semantic_vault.rs")
value = path.read_text(encoding="utf-8")
old = "fn snapshot(state: &AppState) -> Result<SemanticVaultSnapshot> {"
new = "pub(crate) fn snapshot(state: &AppState) -> Result<SemanticVaultSnapshot> {"
if value.count(old) != 1:
    raise SystemExit("semantic vault snapshot visibility target was not found exactly once")
path.write_text(value.replace(old, new, 1), encoding="utf-8")
