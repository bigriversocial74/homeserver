from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def require(text: str, token: str, label: str) -> None:
    if token not in text:
        raise SystemExit(f"Missing {label}: {token}")


def forbid(text: str, token: str, label: str) -> None:
    if token in text:
        raise SystemExit(f"Forbidden {label}: {token}")


migration = read("database/migrations/0030_agent_audio_conversation.sql")
for table in (
    "audio_sessions",
    "audio_segments",
    "conversation_events",
    "audio_permission_receipts",
):
    require(migration, f"CREATE TABLE IF NOT EXISTS {table}", f"{table} schema")
require(migration, "0030_agent_audio_conversation", "migration registration")

runtime = read("crates/homeserver-service/src/audio_runtime.rs")
for route in (
    "/v1/audio/status",
    "/v1/audio/sessions/start",
    "/v1/audio/sessions/state",
    "/v1/audio/sessions/delete",
    "/v1/audio/segments",
    "/v1/audio/segments/transcript",
):
    require(runtime, route, f"protected audio route {route}")
require(runtime, '"raw_audio_persistence": false', "ephemeral raw-audio boundary")
require(runtime, '"cloud_egress": false', "local-only egress boundary")
require(runtime, "another audio session is already active", "single active capture boundary")
require(runtime, "microphone authorization is required", "microphone authorization boundary")
require(runtime, "recording authorization is required", "recording authorization boundary")
require(runtime, "Phase 23A supports ephemeral audio or transcript retention", "raw-audio retention denial")

app = read("crates/homeserver-service/src/app.rs")
for token in (
    '#[path = "audio_runtime.rs"]',
    "audio_runtime::initialize(&connection)?;",
    ".merge(audio_runtime::router(state.clone()))",
    "audio_runtime::maintain_history(&connection)",
):
    require(app, token, "audio service registration")

bridge = read("src-tauri/src/agent.rs")
require(bridge, '"audio".to_owned()', "audio status in Agent workspace")
for action, route in (
    ("audio_status", "/v1/audio/status"),
    ("audio_start_session", "/v1/audio/sessions/start"),
    ("audio_set_state", "/v1/audio/sessions/state"),
    ("audio_finalize_segment", "/v1/audio/segments"),
    ("audio_update_transcript", "/v1/audio/segments/transcript"),
    ("audio_delete_session", "/v1/audio/sessions/delete"),
):
    require(bridge, f'Some("{action}")', f"trusted Tauri action {action}")
    require(bridge, route, f"trusted Tauri route {route}")

index = read("index.html")
require(index, "/src/homeserver-agent-audio.js", "Agent audio module loading")

chat = read("src/homeserver-agent-audio.js")
for token in (
    "navigator.mediaDevices.getUserMedia",
    "new MediaRecorder",
    'audioAction("audio_status")',
    'audioAction("audio_start_session"',
    'audioAction("audio_set_state"',
    'audioAction("audio_finalize_segment"',
    'audioAction("audio_update_transcript"',
    'audioAction("audio_delete_session"',
    "startCapture",
    "stopCapture",
    "finalizeCapture",
    "sendTranscript",
    "raw_audio_retained: false",
    "URL.createObjectURL(blob)",
):
    require(chat, token, "Agent Chat audio integration")
forbid(chat, "SpeechRecognition", "browser/cloud speech recognition")
forbid(chat, "webkitSpeechRecognition", "browser/cloud speech recognition")
forbid(chat, "audio_base64", "raw audio JSON upload")
forbid(chat, "FileReader", "raw audio serialization")

css = read("src/homeserver-agent-audio.css")
require(css, "Phase 23 Agent Chat ears and conversation engine", "audio UI styles")
require(css, ".hs-agent-audio-panel", "audio panel styles")
require(css, ".hs-agent-audio-mic", "microphone control styles")

for temporary_path in (
    ROOT / "src-tauri/src/audio.rs",
    ROOT / "scripts/apply-phase23-agent-audio.py",
    ROOT / ".github/workflows/phase-23-bootstrap.yml",
):
    if temporary_path.exists():
        raise SystemExit(f"Temporary Phase 23 staging file remains: {temporary_path.relative_to(ROOT)}")

print(
    "Phase 23A validates governed Agent Chat microphone capture, ephemeral recordings, "
    "persistent local session/transcript metadata, and conversation-event boundaries."
)
