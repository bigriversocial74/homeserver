#!/usr/bin/env python3
from pathlib import Path

path = Path(__file__).resolve().parents[1] / "crates/homeserver-service/src/review_intelligence.rs"
value = path.read_text(encoding="utf-8")

value = value.replace(
    "#[derive(Debug, Clone, Deserialize)]\nstruct ProviderExportEnvelope {",
    "#[derive(Debug, Clone, Serialize, Deserialize)]\nstruct ProviderExportEnvelope {",
    1,
)
value = value.replace(
    "#[derive(Debug, Clone, Deserialize)]\nstruct ProviderExportRecord {",
    "#[derive(Debug, Clone, Serialize, Deserialize)]\nstruct ProviderExportRecord {",
    1,
)
value = value.replace(
    "#[derive(Debug, Clone, Deserialize)]\nstruct ProviderExportEvent {",
    "#[derive(Debug, Clone, Serialize, Deserialize)]\nstruct ProviderExportEvent {",
    1,
)

old = '''    let mut analysis = tokio::task::spawn_blocking(move || {
        deterministic_analysis(
            &deterministic_state,
            &connection_id,
            &dataset_keys,
            maximum_records,
            &settings,
        )
    })
'''
new = '''    let deterministic_settings = settings.clone();
    let mut analysis = tokio::task::spawn_blocking(move || {
        deterministic_analysis(
            &deterministic_state,
            &connection_id,
            &dataset_keys,
            maximum_records,
            &deterministic_settings,
        )
    })
'''
if old not in value:
    raise SystemExit("settings move repair anchor was not found")
value = value.replace(old, new, 1)

old = '''    let key_name = openai_credential_key(&state.connection()?)?;
'''
new = '''    let key_name = {
        let connection = state.connection()?;
        openai_credential_key(&connection)?
    };
'''
if old not in value:
    raise SystemExit("OpenAI credential key repair anchor was not found")
value = value.replace(old, new, 1)

old = '''            let key = load_openai_key(&state.connection()?)?;
'''
new = '''            let key = {
                let connection = state.connection()?;
                load_openai_key(&connection)?
            };
'''
if old not in value:
    raise SystemExit("OpenAI key load repair anchor was not found")
value = value.replace(old, new, 1)

path.write_text(value, encoding="utf-8", newline="\n")
print("First review intelligence Rust compiler defects repaired.")
