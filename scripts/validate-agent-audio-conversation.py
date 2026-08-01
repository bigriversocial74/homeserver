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

app = read("crates/homeserver-service/src/app.rs")
for token in (
    '#[path = "audio_runtime.rs"]',
    "audio_runtime::initialize(&connection)?;",
    ".merge(audio_runtime::router(state.clone()))",
    "audio_runtime::maintain_history(&connection)",
):
    require(app, token, "audio service registration")

bridge = read("src-tauri/src/audio.rs")
require(bridge, "homeserver_audio_status", "audio status bridge")
require(bridge, "homeserver_audio_action", "audio action bridge")
for action in (
    "start_session",
    "set_state",
    "finalize_segment",
    "update_transcript",
    "delete_session",
):
    require(bridge, f'Some("{action}")', f"Tauri action {action}")

lib = read("src-tauri/src/lib.rs")
require(lib, "mod audio;", "audio command module")
require(lib, "audio::homeserver_audio_status", "audio status handler")
require(lib, "audio::homeserver_audio_action", "audio action handler")

chat = read("src/homeserver-agent-chat.js")
for token in (
    "navigator.mediaDevices.getUserMedia",
    "new MediaRecorder",
    "homeserver_audio_status",
    "homeserver_audio_action",
    "startAudioCapture",
    "stopAudioCapture",
    "finalizeAudioCapture",
    "sendAudioTranscript",
    "raw_audio_retained: false",
):
    require(chat, token, "Agent Chat audio integration")
forbid(chat, "SpeechRecognition", "browser/cloud speech recognition")
forbid(chat, "webkitSpeechRecognition", "browser/cloud speech recognition")
forbid(chat, "audio_base64", "raw audio JSON upload")

css = read("src/homeserver-agent-chat.css")
require(css, "Phase 23 Agent Chat ears and conversation engine", "audio UI styles")
require(css, ".hs-audio-panel", "audio panel styles")
require(css, ".hs-chat-mic", "microphone control styles")

print(
    "Phase 23A validates governed Agent Chat microphone capture, ephemeral recordings, "
    "persistent session/transcript metadata, and conversation-event boundaries."
)
