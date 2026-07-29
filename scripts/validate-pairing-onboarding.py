#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def require(content: str, needle: str, label: str) -> None:
    if needle not in content:
        raise SystemExit(f"pairing onboarding validation failed: missing {label}: {needle}")


index = read("index.html")
default_route = read("src/default-agent-route.js")
onboarding = read("src/pairing-onboarding.js")
activity_ui = read("src/durable-activity-ui.js")
activity_service = read("crates/homeserver-service/src/activity.rs")
service_app = read("crates/homeserver-service/src/app.rs")
tauri_agent = read("src-tauri/src/agent.rs")

require(default_route, '"#agent"', "Agent Workspace default route")
require(index, "/src/default-agent-route.js", "default-route module")
require(index, "/src/pairing-onboarding.js", "pairing onboarding module")
require(index, "/src/durable-activity-ui.js", "durable activity module")
if index.index("/src/default-agent-route.js") > index.index("/src/main.js"):
    raise SystemExit("pairing onboarding validation failed: default route must load before main.js")
if index.index("/src/pairing-onboarding.js") < index.index("/src/homeserver-agent-chat.js"):
    raise SystemExit("pairing onboarding validation failed: onboarding must extend the Agent Chat runtime")

require(onboarding, "https://microgifter.com/account-homeserver.php?source=homeserver-agent", "canonical Microgifter pairing node")
require(onboarding, 'invoke("homeserver_connect_microgifter"', "existing Sync Code exchange command")
require(onboarding, "Open Microgifter Pairing Node", "agent-supplied pairing action")
require(onboarding, "Connect and continue", "same-chat onboarding continuation")
require(onboarding, "pendingApprovals", "approval task notifications")
require(onboarding, "newMessages", "agent message notifications")
require(onboarding, "hs-agent-notification-toggle", "Agent Chat notification control")
require(onboarding, "No onboarding tasks or unread Agent Workspace items require attention", "healthy notification state")

for forbidden in ("access_token=", "device_token=", "entitlement_token=", "private_key="):
    if forbidden in onboarding.lower():
        raise SystemExit(f"pairing onboarding validation failed: permanent secret appears in pairing link: {forbidden}")

require(activity_service, '.route("/v1/activity", get(activity_snapshot))', "activity snapshot route")
require(activity_service, '.route("/v1/activity/active", post(mark_user_active))', "activity mark route")
require(activity_service, "service.started", "durable startup receipt")
require(activity_service, "service.stopped", "durable clean shutdown receipt")
require(activity_service, "control_center.active", "durable user activity receipt")
require(service_app, "activity::initialize(&connection)?;", "activity initialization")
require(service_app, ".merge(activity::router(state.clone()))", "activity API registration")
require(service_app, "activity::record_service_stopped(&state)", "graceful shutdown receipt")
require(tauri_agent, 'get_json::<Value>("/v1/activity")', "activity retrieval through Agent Workspace")
require(tauri_agent, 'post_json("/v1/activity/active"', "user activity marking")
require(activity_ui, "previous_session_clean", "clean versus interrupted session history")
require(activity_ui, "Since you were last active", "durable return briefing")

print("pairing-first Agent Workspace onboarding contract passed")
