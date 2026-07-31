use rusqlite::{params, Connection};

const CORE_MIGRATION: &str =
    include_str!("../../../database/migrations/0020_wrapper_identity_and_pairing.sql");
const GRANT_MIGRATION: &str =
    include_str!("../../../database/migrations/0021_wrapper_capability_grants.sql");
const JOB_MIGRATION: &str =
    include_str!("../../../database/migrations/0022_wrapper_jobs_events_receipts.sql");
const JOB_AUTHORITY_MIGRATION: &str =
    include_str!("../../../database/migrations/0022a_wrapper_job_authority_snapshots.sql");
const AGENT_MIGRATION: &str =
    include_str!("../../../database/migrations/0023_wrapper_agents_and_action_approvals.sql");
const VAULT_MIGRATION: &str = include_str!("../../../database/migrations/0005_knowledge_vault.sql");
const PRIVACY_MIGRATION: &str =
    include_str!("../../../database/migrations/0024_private_knowledge_boundary.sql");
const RUNTIME_MIGRATION: &str =
    include_str!("../../../database/migrations/0025_authorized_agent_tool_runtime.sql");
const ORCHESTRATION_MIGRATION: &str =
    include_str!("../../../database/migrations/0026_supervised_action_orchestration.sql");
const SCHEDULING_MIGRATION: &str =
    include_str!("../../../database/migrations/0027_authorized_agent_scheduling.sql");
const SCHEDULING_SOURCE: &str = include_str!("../src/app/wrapper_scheduling.rs");
const APP_SOURCE: &str = include_str!("../src/app.rs");

fn initialize_contract_database() -> Connection {
    let connection = Connection::open_in_memory().expect("open in-memory database");
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE schema_migrations (
               migration_key TEXT PRIMARY KEY,
               applied_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
             );",
        )
        .expect("initialize migration registry");
    for migration in [
        CORE_MIGRATION,
        GRANT_MIGRATION,
        JOB_MIGRATION,
        JOB_AUTHORITY_MIGRATION,
        AGENT_MIGRATION,
        VAULT_MIGRATION,
        PRIVACY_MIGRATION,
        RUNTIME_MIGRATION,
        ORCHESTRATION_MIGRATION,
        SCHEDULING_MIGRATION,
    ] {
        connection
            .execute_batch(migration)
            .expect("apply contract migration");
    }
    connection
}

#[test]
fn migration_registers_authorized_scheduler_contract() {
    let connection = initialize_contract_database();
    let migration_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE migration_key='0027_authorized_agent_scheduling'",
            [],
            |row| row.get(0),
        )
        .expect("read migration count");
    assert_eq!(migration_count, 1);

    for table in [
        "agent_schedule_definitions",
        "agent_schedule_private_templates",
        "agent_schedule_event_inbox",
        "agent_schedule_cursors",
        "agent_schedule_runs",
        "agent_schedule_receipts",
        "agent_schedule_audit_events",
        "agent_scheduler_state",
    ] {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![table],
                |row| row.get(0),
            )
            .expect("read table contract");
        assert_eq!(count, 1, "missing {table}");
    }

    for trigger in [
        "trg_agent_schedule_event_inbox_no_update",
        "trg_agent_schedule_event_inbox_no_delete",
        "trg_agent_schedule_receipts_no_update",
        "trg_agent_schedule_receipts_no_delete",
        "trg_agent_schedule_audit_no_update",
        "trg_agent_schedule_audit_no_delete",
        "trg_agent_schedule_definitions_immutable_fields",
        "trg_agent_schedule_definitions_no_delete",
        "trg_agent_schedule_private_templates_no_update",
        "trg_agent_schedule_private_templates_no_delete",
        "trg_agent_schedule_runs_terminal_no_update",
        "trg_agent_schedule_runs_no_delete",
    ] {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name=?1",
                params![trigger],
                |row| row.get(0),
            )
            .expect("read trigger contract");
        assert_eq!(count, 1, "missing {trigger}");
    }
}

#[test]
fn immutable_schedule_evidence_rejects_update_and_delete() {
    let connection = initialize_contract_database();
    connection
        .pragma_update(None, "foreign_keys", false)
        .expect("disable foreign keys for trigger-only fixture");
    connection
        .execute_batch(
            "INSERT INTO agent_schedule_definitions (
               schedule_id,agent_id,agent_revision,assignment_id,assignment_revision,
               wrapper_id,connection_id,connection_authority_revision,created_by_user_id,
               title,description,state,trigger_kind,run_at_utc,misfire_policy,overlap_policy,
               debounce_seconds,max_runs,run_count,template_hash,authority_snapshot_json,
               authority_hash,next_fire_at_utc,expires_at_utc,created_at_utc,updated_at_utc
             ) VALUES (
               '11111111-1111-4111-8111-111111111111',
               '22222222-2222-4222-8222-222222222222',1,
               '33333333-3333-4333-8333-333333333333',1,
               '44444444-4444-4444-8444-444444444444',
               '55555555-5555-4555-8555-555555555555',1,'local_control_center',
               'Fixture','', 'active','one_time','2099-01-01T00:00:00.000Z',
               'fire_once','skip',0,1,0,
               'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
               '{}',
               'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
               '2099-01-01T00:00:00.000Z','2099-01-02T00:00:00.000Z',
               '2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z'
             );
             INSERT INTO agent_schedule_private_templates (
               schedule_id,classification,template_json,template_bytes,created_at_utc
             ) VALUES (
               '11111111-1111-4111-8111-111111111111','private','{}',2,
               '2026-01-01T00:00:00.000Z'
             );
             INSERT INTO agent_schedule_event_inbox (
               event_id,topic,source_type,source_id,event_key,safe_metadata_json,
               payload_hash,occurred_at_utc,received_at_utc
             ) VALUES (
               '66666666-6666-4666-8666-666666666666','runtime.plan.completed',
               'runtime','fixture','event-key','{}',
               'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
               '2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z'
             );
             INSERT INTO agent_schedule_runs (
               run_id,schedule_id,trigger_kind,trigger_token,scheduled_for_utc,state,
               authority_hash,template_hash,outcome,result_code,created_at_utc,completed_at_utc
             ) VALUES (
               '77777777-7777-4777-8777-777777777777',
               '11111111-1111-4111-8111-111111111111','one_time',
               'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
               '2026-01-01T00:00:00.000Z','completed',
               'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
               'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
               'completed','runtime_plan_created',
               '2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z'
             );
             INSERT INTO agent_schedule_receipts (
               receipt_id,schedule_id,run_id,agent_id,assignment_id,wrapper_id,connection_id,
               trigger_kind,trigger_token,outcome,result_code,authority_hash,template_hash,
               receipt_hash,completed_at_utc
             ) VALUES (
               '88888888-8888-4888-8888-888888888888',
               '11111111-1111-4111-8111-111111111111',
               '77777777-7777-4777-8777-777777777777',
               '22222222-2222-4222-8222-222222222222',
               '33333333-3333-4333-8333-333333333333',
               '44444444-4444-4444-8444-444444444444',
               '55555555-5555-4555-8555-555555555555',
               'one_time',
               'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
               'completed','runtime_plan_created',
               'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
               'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
               'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
               '2026-01-01T00:00:00.000Z'
             );
             INSERT INTO agent_schedule_audit_events (
               audit_event_id,schedule_id,run_id,event_type,outcome,actor_type,actor_id,
               detail_code,metadata_json,event_hash,created_at_utc
             ) VALUES (
               '99999999-9999-4999-8999-999999999999',
               '11111111-1111-4111-8111-111111111111',
               '77777777-7777-4777-8777-777777777777',
               'agent.schedule_plan_created','success','scheduler','agent_scheduler',
               'phase17_plan_created','{}',
               'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff',
               '2026-01-01T00:00:00.000Z'
             );",
        )
        .expect("insert trigger fixtures");

    for sql in [
        "UPDATE agent_schedule_event_inbox SET source_id='changed' WHERE event_id='66666666-6666-4666-8666-666666666666'",
        "DELETE FROM agent_schedule_event_inbox WHERE event_id='66666666-6666-4666-8666-666666666666'",
        "UPDATE agent_schedule_receipts SET result_code='changed' WHERE receipt_id='88888888-8888-4888-8888-888888888888'",
        "DELETE FROM agent_schedule_receipts WHERE receipt_id='88888888-8888-4888-8888-888888888888'",
        "UPDATE agent_schedule_audit_events SET detail_code='changed' WHERE audit_event_id='99999999-9999-4999-8999-999999999999'",
        "DELETE FROM agent_schedule_audit_events WHERE audit_event_id='99999999-9999-4999-8999-999999999999'",
        "UPDATE agent_schedule_definitions SET authority_hash='dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd' WHERE schedule_id='11111111-1111-4111-8111-111111111111'",
        "DELETE FROM agent_schedule_definitions WHERE schedule_id='11111111-1111-4111-8111-111111111111'",
        r#"UPDATE agent_schedule_private_templates SET template_json='{"changed":true}' WHERE schedule_id='11111111-1111-4111-8111-111111111111'"#,
        "DELETE FROM agent_schedule_private_templates WHERE schedule_id='11111111-1111-4111-8111-111111111111'",
        "UPDATE agent_schedule_runs SET result_code='changed' WHERE run_id='77777777-7777-4777-8777-777777777777'",
        "DELETE FROM agent_schedule_runs WHERE run_id='77777777-7777-4777-8777-777777777777'",
    ] {
        assert!(connection.execute(sql, []).is_err(), "{sql} must fail");
    }
}

#[test]
fn scheduler_preserves_phase_16_through_18_authority_boundaries() {
    for required in [
        "capture_authority",
        "revalidate_authority",
        "wrapper_runtime::create_plan",
        "agent_scheduler:",
        "schedule capability binding, grant, or execution policy changed",
        "phase17_runtime_required: true",
        "phase18_supervision_required: true",
        "private_templates_exposed: false",
        "private_event_payloads_exposed: false",
        "direct_execution_allowed: false",
        "trigger_token",
        "misfire_policy",
        "overlap_policy",
        "debounce_seconds",
        "reconcile_interrupted_runs",
        "runtime_plan_recovered",
        "retention requires archival",
        "SELECT COALESCE(MAX(event_sequence),0)",
        "safe event source type does not match its topic",
        "safe event metadata field is not allowed for this topic",
        "safe event metadata values must be primitive",
        "schedule changed during runtime plan creation",
        "wrapper_runtime::cancel_plan",
        "state='creating_plan'",
        "state='queued'",
        "schedule authority is blocked by an active emergency stop",
        "wrapper_runtime::cancel_plan_as_system",
    ] {
        assert!(SCHEDULING_SOURCE.contains(required), "missing {required}");
    }
    for forbidden in [
        "execute_adapter(",
        "execute_proposal_as_orchestrator(",
        "tokio::process::Command",
        "std::process::Command",
        "powershell",
        "cmd.exe",
        "webhook",
    ] {
        assert!(
            !SCHEDULING_SOURCE.contains(forbidden),
            "forbidden {forbidden}"
        );
    }
    assert!(APP_SOURCE.contains("wrapper_scheduling::initialize"));
    assert!(APP_SOURCE.contains("wrapper_scheduling::run"));
    assert!(APP_SOURCE.contains(".merge(wrapper_scheduling::router"));
}

#[test]
fn event_topics_and_private_fields_are_closed() {
    for topic in [
        "wrapper.job.completed",
        "runtime.plan.completed",
        "supervised.action.completed",
        "cloud.sync.completed",
    ] {
        assert!(SCHEDULING_SOURCE.contains(topic));
    }
    for private_key in [
        "private_input",
        "private_result",
        "credential",
        "api_key",
        "secret",
        "raw_prompt",
        "conversation",
    ] {
        assert!(SCHEDULING_SOURCE.contains(private_key));
    }
    assert!(SCHEDULING_SOURCE.contains("safe event metadata contains a forbidden private field"));
    assert!(SCHEDULING_MIGRATION.contains("classification TEXT NOT NULL DEFAULT 'private'"));
    assert!(!SCHEDULING_MIGRATION.contains("DELETE FROM agent_schedule_event_inbox"));
    assert!(!SCHEDULING_MIGRATION.contains("DELETE FROM agent_schedule_receipts"));
}
