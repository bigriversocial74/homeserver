#!/usr/bin/env python3
"""Apply focused Rust quality repairs after the temporary Phase 5C integration patch."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_once(value: str, old: str, new: str, label: str) -> str:
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, found {count}")
    return value.replace(old, new, 1)


operational_path = ROOT / "crates/homeserver-service/src/operational_data.rs"
operational = operational_path.read_text(encoding="utf-8")
operational = replace_once(
    operational,
    "use std::{collections::HashSet, sync::Arc};",
    "use std::sync::Arc;",
    "unused operational import",
)
operational_path.write_text(operational, encoding="utf-8", newline="\n")

agent_path = ROOT / "crates/homeserver-service/src/agent_runtime.rs"
agent = agent_path.read_text(encoding="utf-8")
agent = replace_once(
    agent,
    '''    let assistant_text = generate_grounded_response(
        &request,
        &selected_connections,
        &selected_goals,
        knowledge.as_ref(),
        operational.as_ref(),
        models.as_ref(),
        plan.as_ref(),
        mission.as_ref(),
    )
    .await?;
''',
    '''    let assistant_text = generate_grounded_response(
        &request,
        GroundedResponseContext {
            connections: &selected_connections,
            goals: &selected_goals,
            knowledge: knowledge.as_ref(),
            operational: operational.as_ref(),
            models: models.as_ref(),
            plan: plan.as_ref(),
            mission: mission.as_ref(),
        },
    )
    .await?;
''',
    "grounded response call",
)
agent = replace_once(
    agent,
    '''async fn generate_grounded_response(
    request: &AgentPromptRequest,
    connections: &[cloud_registry::CloudConnectionSummary],
    goals: &[AgentGoalSummary],
    knowledge: Option<&semantic_vault::SemanticSearchResult>,
    operational: Option<&operational_data::OperationalQueryResult>,
    models: Option<&model_center::ModelCenterSnapshot>,
    plan: Option<&AgentPlanSummary>,
    mission: Option<&WorldMissionSummary>,
) -> Result<String> {
''',
    '''struct GroundedResponseContext<'a> {
    connections: &'a [cloud_registry::CloudConnectionSummary],
    goals: &'a [AgentGoalSummary],
    knowledge: Option<&'a semantic_vault::SemanticSearchResult>,
    operational: Option<&'a operational_data::OperationalQueryResult>,
    models: Option<&'a model_center::ModelCenterSnapshot>,
    plan: Option<&'a AgentPlanSummary>,
    mission: Option<&'a WorldMissionSummary>,
}

async fn generate_grounded_response(
    request: &AgentPromptRequest,
    context: GroundedResponseContext<'_>,
) -> Result<String> {
    let GroundedResponseContext {
        connections,
        goals,
        knowledge,
        operational,
        models,
        plan,
        mission,
    } = context;
''',
    "grounded response signature",
)
agent_path.write_text(agent, encoding="utf-8", newline="\n")

print("Phase 5C Rust quality repairs applied.")
