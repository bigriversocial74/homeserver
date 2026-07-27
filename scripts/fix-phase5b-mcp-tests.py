#!/usr/bin/env python3
"""Update MCP unit tests for the supervised read/request-only Phase 5B contract."""
from pathlib import Path

path = Path(__file__).resolve().parents[1] / "crates/homeserver-service/src/mcp_runtime.rs"
content = path.read_text(encoding="utf-8")
old = r'''    #[test]
    fn scopes_are_read_only_and_deduplicated() {
        let scopes = normalize_scopes(&[
            "knowledge.read".to_owned(),
            "system.read".to_owned(),
            "knowledge.read".to_owned(),
        ])
        .expect("read-only scopes should be accepted");
        assert_eq!(scopes, vec!["knowledge.read", "system.read"]);
        assert!(normalize_scopes(&["models.write".to_owned()]).is_err());
    }
'''
new = r'''    #[test]
    fn scopes_are_supervised_and_deduplicated() {
        let scopes = normalize_scopes(&[
            "knowledge.read".to_owned(),
            "system.read".to_owned(),
            "agents.request".to_owned(),
            "knowledge.read".to_owned(),
        ])
        .expect("read and request-only scopes should be accepted");
        assert_eq!(
            scopes,
            vec!["agents.request", "knowledge.read", "system.read"]
        );
        assert!(normalize_scopes(&["models.write".to_owned()]).is_err());
        assert!(normalize_scopes(&["agents.execute".to_owned()]).is_err());
    }
'''
if content.count(old) != 1:
    raise SystemExit("MCP scope test anchor was not found exactly once")
content = content.replace(old, new, 1)
old = r'''    #[test]
    fn tools_are_marked_read_only() {
        let scopes = ALLOWED_SCOPES
            .iter()
            .map(|scope| (*scope).to_owned())
            .collect::<HashSet<_>>();
        let tools = tool_definitions(&scopes);
        assert_eq!(tools.len(), 5);
        assert!(tools.iter().all(|tool| {
            tool.pointer("/annotations/readOnlyHint") == Some(&Value::Bool(true))
                && tool.pointer("/annotations/destructiveHint") == Some(&Value::Bool(false))
        }));
    }
'''
new = r'''    #[test]
    fn tools_are_marked_read_or_request_only() {
        let scopes = ALLOWED_SCOPES
            .iter()
            .map(|scope| (*scope).to_owned())
            .collect::<HashSet<_>>();
        let tools = tool_definitions(&scopes);
        assert_eq!(tools.len(), 14);
        let request_tools = [
            "homeserver_agent_prompt",
            "homeserver_agent_plan_submit",
            "homeserver_agent_plan_cancel",
            "homeserver_world_mission_draft",
        ]
        .into_iter()
        .collect::<HashSet<_>>();
        for tool in &tools {
            let name = tool.get("name").and_then(Value::as_str).unwrap();
            assert_eq!(
                tool.pointer("/annotations/destructiveHint"),
                Some(&Value::Bool(false))
            );
            assert_eq!(
                tool.pointer("/annotations/openWorldHint"),
                Some(&Value::Bool(false))
            );
            if request_tools.contains(name) {
                assert_eq!(
                    tool.pointer("/annotations/readOnlyHint"),
                    Some(&Value::Bool(false))
                );
                assert_eq!(
                    tool.pointer("/annotations/requestOnly"),
                    Some(&Value::Bool(true))
                );
            } else {
                assert_eq!(
                    tool.pointer("/annotations/readOnlyHint"),
                    Some(&Value::Bool(true))
                );
            }
        }
        let names = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<HashSet<_>>();
        assert!(!names.contains("homeserver_agent_plan_approve"));
        assert!(!names.contains("homeserver_agent_plan_execute"));
        assert!(!names.contains("homeserver_world_mission_dispatch"));
    }
'''
if content.count(old) != 1:
    raise SystemExit("MCP tool annotation test anchor was not found exactly once")
path.write_text(content.replace(old, new, 1), encoding="utf-8", newline="\n")
print("MCP supervised unit tests updated.")
