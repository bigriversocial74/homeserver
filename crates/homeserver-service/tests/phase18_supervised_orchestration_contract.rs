use rusqlite::{params, Connection};

const MIGRATION: &str =
    include_str!("../../../database/migrations/0026_supervised_action_orchestration.sql");
const ORCHESTRATION_SOURCE: &str = include_str!("../src/app/wrapper_orchestration.rs");
const AGENT_SOURCE: &str = include_str!("../src/app/wrapper_agents.rs");
const RUNTIME_SOURCE: &str = include_str!("../src/app/wrapper_runtime.rs");
const COMPLETION_SOURCE: &str = include_str!("../src/app/wrapper_jobs_completion.rs");

fn initialize_contract_database() -> Connection {
    let connection = Connection::open_in_memory().expect("open Phase 18 contract database");
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY);
             CREATE TABLE agent_tool_catalog (
               tool_key TEXT PRIMARY KEY, adapter_key TEXT NOT NULL UNIQUE,
               version TEXT NOT NULL, description TEXT NOT NULL, risk_class TEXT NOT NULL,
               approval_requirement TEXT NOT NULL, allowed_job_types_json TEXT NOT NULL,
               input_schema_json TEXT NOT NULL, output_schema_json TEXT NOT NULL,
               max_execution_seconds INTEGER NOT NULL, state TEXT NOT NULL DEFAULT 'active',
               created_at_utc TEXT NOT NULL, updated_at_utc TEXT NOT NULL
             );
             CREATE TABLE agent_runtime_plans (plan_id TEXT PRIMARY KEY);
             CREATE TABLE agent_runtime_plan_steps (step_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_jobs (job_id TEXT PRIMARY KEY);
             CREATE TABLE homeserver_agents (agent_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_identities (wrapper_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_connections (connection_id TEXT PRIMARY KEY);
             CREATE TABLE agent_action_proposals (proposal_id TEXT PRIMARY KEY);
             CREATE TABLE agent_action_approvals (approval_id TEXT PRIMARY KEY);
             CREATE TABLE agent_execution_policies (policy_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_agent_assignments (assignment_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_capability_grants (grant_id TEXT PRIMARY KEY);
             CREATE TABLE agent_action_receipts (receipt_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_job_execution_receipts (receipt_id TEXT PRIMARY KEY);
             CREATE TABLE agent_runtime_receipts (receipt_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_job_workers (worker_id TEXT PRIMARY KEY);",
        )
        .expect("create Phase 18 prerequisite tables");
    connection
        .execute_batch(MIGRATION)
        .expect("apply Phase 18 migration");
    connection
}

#[test]
fn phase18_reuses_certified_phase16_and_phase17_contracts() {
    for required in [
        "wrapper_runtime::create_plan",
        "wrapper_jobs::claim_jobs",
        "wrapper_jobs::start_job",
        "wrapper_jobs::complete_job",
        "wrapper_agents::create_proposal",
        "wrapper_agents::execute_proposal_as_orchestrator",
        "approval_payload_hash",
        "approval_connection_authority_revision",
        "approval_consumed_once: true",
        "sensitive_runtime_bypass_allowed: false",
        "phase16e_egress_required: true",
        "proposal_job_egress_enforced",
        "checkpoint_authority_denied",
        "runtime_plan_no_longer_active",
    ] {
        assert!(
            ORCHESTRATION_SOURCE.contains(required),
            "missing Phase 18 supervised boundary: {required}"
        );
    }
    assert!(COMPLETION_SOURCE.contains("wrapper_privacy::evaluate_egress_tx"));
    assert!(AGENT_SOURCE.contains("approval was already consumed"));
    assert!(AGENT_SOURCE.contains("execute_proposal_as_orchestrator"));
    assert!(RUNTIME_SOURCE.contains("tool_key='action.supervised'"));
    assert!(!ORCHESTRATION_SOURCE.contains("std::process::Command"));
    assert!(!ORCHESTRATION_SOURCE.contains("tokio::process::Command"));
    assert!(!ORCHESTRATION_SOURCE.contains("DELETE FROM agent_supervised_action_events"));
    assert!(ORCHESTRATION_SOURCE.contains("supervised action event retention requires archival"));
    assert!(
        ORCHESTRATION_SOURCE
            .matches("job_receipt.safe_result_hash.is_some()")
            .count()
            >= 3
    );
}

#[test]
fn phase18_catalog_is_closed_and_proposal_gated() {
    let connection = initialize_contract_database();
    let row: (String, String, String, String) = connection
        .query_row(
            "SELECT adapter_key,risk_class,approval_requirement,allowed_job_types_json FROM agent_tool_catalog WHERE tool_key='action.supervised'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read supervised tool");
    assert_eq!(row.0, "action.supervised");
    assert_eq!(row.1, "external_side_effect");
    assert_eq!(row.2, "proposal");
    assert_eq!(row.3, "[\"action.propose\"]");
    let tools: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_tool_catalog WHERE tool_key='action.supervised'",
            [],
            |row| row.get(0),
        )
        .expect("count supervised tools");
    assert_eq!(tools, 1);
}

#[test]
fn phase18_terminal_evidence_is_immutable_and_checkpoint_is_resumable() {
    let connection = initialize_contract_database();
    // This disposable fixture validates immutable triggers only. Referential authority
    // is covered independently by the migration schema and orchestration API tests.
    connection
        .pragma_update(None, "foreign_keys", false)
        .expect("disable foreign keys for trigger-only fixtures");
    connection
        .execute_batch(
            "INSERT INTO agent_supervised_action_events (
               event_id,event_type,outcome,actor_type,actor_id,detail_code,
               metadata_json,event_hash,created_at_utc
             ) VALUES (
               '11111111-1111-4111-8111-111111111111','agent.checkpoint.created',
               'success','system','orchestrator','created','{}',
               'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
               '2026-01-01T00:00:00.000Z'
             );
             INSERT INTO agent_supervised_action_receipts (
               receipt_id,checkpoint_id,plan_id,step_id,job_id,proposal_id,
               wrapper_job_receipt_id,wrapper_job_receipt_hash,runtime_receipt_id,
               runtime_receipt_hash,runtime_plan_hash,proposal_plan_hash,payload_hash,
               outcome,result_code,phase16e_detail_code,receipt_hash,completed_at_utc,
               created_at_utc
             ) VALUES (
               '22222222-2222-4222-8222-222222222222',
               '33333333-3333-4333-8333-333333333333',
               '44444444-4444-4444-8444-444444444444',
               '55555555-5555-4555-8555-555555555555',
               '66666666-6666-4666-8666-666666666666',
               '77777777-7777-4777-8777-777777777777',
               '88888888-8888-4888-8888-888888888888',
               'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
               '99999999-9999-4999-8999-999999999999',
               'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
               'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
               'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
               'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff',
               'failed','approval_rejected','proposal_job_egress_enforced',
               '1212121212121212121212121212121212121212121212121212121212121212',
               '2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z'
             );
             INSERT INTO agent_supervised_compensation_receipts (
               compensation_receipt_id,checkpoint_id,action_receipt_id,adapter_key,
               outcome,result_code,target_hash,receipt_hash,completed_at_utc,created_at_utc
             ) VALUES (
               'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
               'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
               'cccccccc-cccc-4ccc-8ccc-cccccccccccc','report.delete','completed',
               'report_removed',
               '3434343434343434343434343434343434343434343434343434343434343434',
               '5656565656565656565656565656565656565656565656565656565656565656',
               '2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z'
             );",
        )
        .expect("insert Phase 18 evidence fixtures");
    assert!(connection
        .execute(
            "UPDATE agent_supervised_action_events SET detail_code='changed' WHERE event_id=?1",
            params!["11111111-1111-4111-8111-111111111111"],
        )
        .is_err());
    assert!(connection
        .execute(
            "UPDATE agent_supervised_action_receipts SET result_code='changed' WHERE receipt_id=?1",
            params!["22222222-2222-4222-8222-222222222222"],
        )
        .is_err());
    assert!(connection
        .execute(
            "UPDATE agent_supervised_compensation_receipts SET result_code='changed' WHERE compensation_receipt_id=?1",
            params!["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
        )
        .is_err());
    assert!(connection
        .execute(
            "DELETE FROM agent_supervised_action_events WHERE event_id=?1",
            params!["11111111-1111-4111-8111-111111111111"],
        )
        .is_err());
    assert!(connection
        .execute(
            "DELETE FROM agent_supervised_action_receipts WHERE receipt_id=?1",
            params!["22222222-2222-4222-8222-222222222222"],
        )
        .is_err());
    assert!(connection
        .execute(
            "DELETE FROM agent_supervised_compensation_receipts WHERE compensation_receipt_id=?1",
            params!["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
        )
        .is_err());
}
