#!/usr/bin/env python3
"""One-time branch repair for the POD capability-test json serializer."""
from pathlib import Path

PATH = Path("crates/homeserver-service/src/app/pod_provider_runtime.rs")
lines = PATH.read_text(encoding="utf-8").splitlines()
opening = '        "capability_test" => Ok(('
next_arm = '        "speech_to_text" => {'

if opening not in lines:
    if any(line == '        "capability_test" => {' for line in lines):
        print("POD capability-test serializer is already repaired.")
        raise SystemExit(0)
    raise SystemExit("POD capability-test opening arm was not found.")

start = lines.index(opening)
try:
    end = lines.index(next_arm, start + 1)
except ValueError as error:
    raise SystemExit("POD speech-to-text arm was not found after capability test.") from error

expected_models = '                "models": [runtime.transcription_model.clone(), runtime.synthesis_model.clone()].into_iter().flatten().collect::<Vec<_>>(),'
if expected_models not in lines[start:end]:
    raise SystemExit("POD capability-test model expression was not found.")

replacement = [
    '        "capability_test" => {',
    '            let models = [',
    '                runtime.transcription_model.clone(),',
    '                runtime.synthesis_model.clone(),',
    '            ]',
    '            .into_iter()',
    '            .flatten()',
    '            .collect::<Vec<_>>();',
    '            Ok((',
    '                json!({',
    '                    "runtime": "homeserver-local-command-v1",',
    '                    "models": models,',
    '                    "transcription_ready": runtime.transcription_enabled && executable_ready(runtime.transcription_executable.as_deref()),',
    '                    "synthesis_ready": runtime.synthesis_enabled && executable_ready(runtime.synthesis_executable.as_deref()),',
    '                    "details": runtime.runtime_health_message,',
    '                }),',
    '                None,',
    '                Some(started.elapsed().as_millis() as u64),',
    '            ))',
    '        },',
]
PATH.write_text("\n".join(lines[:start] + replacement + lines[end:]) + "\n", encoding="utf-8")
print("POD capability-test serializer repaired.")
