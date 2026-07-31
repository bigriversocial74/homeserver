from pathlib import Path

path = Path("crates/homeserver-service/src/agent_integrations.rs")
value = path.read_text(encoding="utf-8")
replacements = {
    "tokio::task::spawn_blocking(move || read_integrations(&task_state.connection()?))": "tokio::task::spawn_blocking(move || {\n            let connection = task_state.connection()?;\n            read_integrations(&connection)\n        })",
    "let installation_id = crate::database::installation_id(&state.connection()?)?;": "let installation_id = {\n        let connection = state.connection()?;\n        crate::database::installation_id(&connection)?\n    };",
    "Ok(integration_by_id(&state.connection()?, connection_id)?.summary)": "let connection = state.connection()?;\n            Ok(integration_by_id(&connection, connection_id)?.summary)",
}
for old, new in replacements.items():
    if value.count(old) != 1:
        raise SystemExit(f"connection guard repair target not found exactly once: {old}")
    value = value.replace(old, new, 1)
path.write_text(value, encoding="utf-8")
