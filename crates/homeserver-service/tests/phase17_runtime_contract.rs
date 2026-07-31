use rusqlite::{params, Connection};

const MIGRATION: &str =
    include_str!("../../../database/migrations/0025_authorized_agent_tool_runtime.sql");
const RUNTIME_SOURCE: &str = include_str!("../src/app/wrapper_runtime.rs");
const POLICY_SOURCE: &str = include_str!("../src/app/wrapper_runtime_policy.rs");
const COMPLETION_SOURCE: &str = include_str!("../src/app/wrapper_jobs_completion.rs");

fn initialize_contract_database() -> Connection {
    let connection = Connection::open_in_memory().expect("open Phase 17 contract database");
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY);
             CREATE TABLE wrapper_identities (wrapper_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_connections (
               connection_id TEXT PRIMARY KEY,
               wrapper_id TEXT NOT NULL,
               FOREIGN KEY(wrapper_id) REFERENCES wrapper_identities(wrapper_id)
             );
             CREATE TABLE wrapper_job_workers (
               worker_id TEXT PRIMARY KEY,
               worker_kind TEXT NOT NULL
             );
             CREATE TABLE homeserver_agents (agent_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_jobs (job_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_job_execution_receipts (receipt_id TEXT PRIMARY KEY);",
        )
        .expect("create Phase 17 prerequisite tables");
    connection
        .execute_batch(MIGRATION)
        .expect("apply Phase 17 migration");
    connection
}

#[test]
fn phase17_runtime_retains_certified_authority_chain() {
    for required in [
        "wrapper_jobs::submit_job",
        "wrapper_jobs::claim_jobs",
        "wrapper_jobs::start_job",
        "wrapper_jobs::complete_job",
        "wrapper_jobs::fail_job",
        "wrapper_jobs::cancel_job",
        "wrapper_agents::agent_job_authority_is_current_tx",
        "runtime policy execution limit reached",
        "runtime plan step cannot execute before its predecessors",
        "step.job.approval_id.is_none() && step.job.plan_hash.is_none()",
        "private_inputs_exposed: false",
        "private_results_exposed: false",
        "direct_tool_bypass_allowed: false",
        "phase16e_egress_required: true",
    ] {
        assert!(
            RUNTIME_SOURCE.contains(required),
            "missing retained Phase 17 authority boundary: {required}"
        );
    }
    assert!(COMPLETION_SOURCE.contains("wrapper_privacy::evaluate_egress_tx"));
    assert!(!RUNTIME_SOURCE.contains("std::process::Command"));
    assert!(!RUNTIME_SOURCE.contains("tokio::process::Command"));
    assert!(!RUNTIME_SOURCE.contains("agent_action_approvals"));
}

#[test]
fn phase17_policy_is_catalog_bound_and_low_risk() {
    for required in [
        "SELECT adapter_key,risk_class,approval_requirement,state FROM agent_tool_catalog",
        "Phase 17 runtime policies must be approval-free and low-risk",
        "proposal-gated tools are not executable in the Phase 17 runtime",
        "approval-free runtime policy requires scoped autonomy",
    ] {
        assert!(
            POLICY_SOURCE.contains(required),
            "missing Phase 17 policy boundary: {required}"
        );
    }
    let request_start = POLICY_SOURCE
        .find("pub struct CreateRuntimePolicyRequest")
        .expect("runtime policy request exists");
    let response_start = POLICY_SOURCE[request_start..]
        .find("pub struct RuntimePolicyResponse")
        .map(|offset| request_start + offset)
        .expect("runtime policy response exists");
    let request_contract = &POLICY_SOURCE[request_start..response_start];
    assert!(!request_contract.contains("tool_adapter"));
    assert!(!request_contract.contains("risk_class"));
}

#[test]
fn phase17_migration_seeds_only_closed_world_tools() {
    let connection = initialize_contract_database();
    let mut statement = connection
        .prepare("SELECT tool_key FROM agent_tool_catalog ORDER BY tool_key")
        .expect("prepare tool catalog query");
    let tools = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query tool catalog")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect tool catalog");
    assert_eq!(
        tools,
        vec![
            "audit.record",
            "receipt.read",
            "result.compose",
            "wrapper.status.read"
        ]
    );
    let sensitive: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_tool_catalog WHERE risk_class IN ('external_side_effect','high_risk') OR approval_requirement='proposal'",
            [],
            |row| row.get(0),
        )
        .expect("count sensitive runtime tools");
    assert_eq!(sensitive, 0);
}

#[test]
fn phase17_events_and_receipts_are_immutable() {
    let connection = initialize_contract_database();
    connection
        .execute_batch(
            "INSERT INTO wrapper_identities VALUES ('11111111-1111-4111-8111-111111111111');
             INSERT INTO wrapper_connections VALUES (
               '22222222-2222-4222-8222-222222222222',
               '11111111-1111-4111-8111-111111111111'
             );
             INSERT INTO wrapper_job_workers VALUES (
               '33333333-3333-4333-8333-333333333333','tool'
             );
             INSERT INTO homeserver_agents VALUES ('44444444-4444-4444-8444-444444444444');
             INSERT INTO wrapper_jobs VALUES ('55555555-5555-4555-8555-555555555555');
             INSERT INTO agent_runtime_plans (
               plan_id,agent_id,requested_by_user_id,title,objective,state,step_count,
               correlation_id,plan_hash,expires_at_utc,created_at_utc,updated_at_utc
             ) VALUES (
               '66666666-6666-4666-8666-666666666666',
               '44444444-4444-4444-8444-444444444444','owner','Plan','Objective',
               'running',1,'77777777-7777-4777-8777-777777777777',
               'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
               '2099-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z',
               '2026-01-01T00:00:00.000Z'
             );
             INSERT INTO agent_runtime_plan_steps (
               step_id,plan_id,sequence_number,job_id,tool_key,adapter_key,action_type,
               state,idempotency_key,argument_hash,created_at_utc,updated_at_utc
             ) VALUES (
               '88888888-8888-4888-8888-888888888888',
               '66666666-6666-4666-8666-666666666666',1,
               '55555555-5555-4555-8555-555555555555','audit.record','audit.record',
               'audit.record','running','phase17-test-key',
               'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
               '2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z'
             );
             INSERT INTO agent_runtime_events (
               event_id,plan_id,step_id,job_id,agent_id,event_type,outcome,actor_type,
               actor_id,detail_code,metadata_json,event_hash,created_at_utc
             ) VALUES (
               '99999999-9999-4999-8999-999999999999',
               '66666666-6666-4666-8666-666666666666',
               '88888888-8888-4888-8888-888888888888',
               '55555555-5555-4555-8555-555555555555',
               '44444444-4444-4444-8444-444444444444',
               'agent.runtime_step_started','success','worker','runtime','test','{}',
               'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
               '2026-01-01T00:00:00.000Z'
             );",
        )
        .expect("insert runtime evidence fixtures");
    connection
        .execute(
            "INSERT INTO agent_runtime_receipts (
               receipt_id,plan_id,step_id,job_id,agent_id,wrapper_id,connection_id,
               tool_key,adapter_key,outcome,result_code,runtime_receipt_hash,
               completed_at_utc,created_at_utc
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'failed','test_failure',?10,?11,?11)",
            params![
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                "66666666-6666-4666-8666-666666666666",
                "88888888-8888-4888-8888-888888888888",
                "55555555-5555-4555-8555-555555555555",
                "44444444-4444-4444-8444-444444444444",
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222",
                "audit.record",
                "audit.record",
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                "2026-01-01T00:00:00.000Z"
            ],
        )
        .expect("insert runtime receipt fixture");
    assert!(connection
        .execute(
            "UPDATE agent_runtime_events SET detail_code='changed' WHERE event_id=?1",
            params!["99999999-9999-4999-8999-999999999999"],
        )
        .is_err());
    assert!(connection
        .execute(
            "UPDATE agent_runtime_receipts SET result_code='changed' WHERE receipt_id=?1",
            params!["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
        )
        .is_err());
}
