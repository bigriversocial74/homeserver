use rusqlite::{params, Connection};

const MIGRATION: &str =
    include_str!("../../../database/migrations/0028_authorized_model_routing.sql");

fn database() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database");
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY);
             CREATE TABLE homeserver_agents (agent_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_agent_assignments (assignment_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_identities (wrapper_id TEXT PRIMARY KEY);
             CREATE TABLE wrapper_connections (connection_id TEXT PRIMARY KEY);
             CREATE TABLE private_resource_selectors (selector_id TEXT PRIMARY KEY);",
        )
        .expect("prerequisite schema");
    connection.execute_batch(MIGRATION).expect("Phase 20 migration");
    connection
}

#[test]
fn default_policy_is_local_only_and_remote_denied() {
    let connection = database();
    let (subject, providers, remote, fallback): (String, String, String, i64) = connection
        .query_row(
            "SELECT subject_type,provider_order_json,remote_context_mode,allow_fallback
             FROM model_routing_policies
             WHERE policy_id='00000000-0000-4000-8000-000000000020'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("default policy");
    assert_eq!(subject, "local_control_center");
    assert_eq!(providers, "[\"ollama\"]");
    assert_eq!(remote, "deny");
    assert_eq!(fallback, 0);
}

#[test]
fn policy_authority_rejects_update_and_delete() {
    let connection = database();
    assert!(connection
        .execute(
            "UPDATE model_routing_policies SET provider_order_json='[\"openrouter\"]'
             WHERE policy_id='00000000-0000-4000-8000-000000000020'",
            [],
        )
        .is_err());
    assert!(connection
        .execute(
            "DELETE FROM model_routing_policies
             WHERE policy_id='00000000-0000-4000-8000-000000000020'",
            [],
        )
        .is_err());
}

fn insert_terminal_fixture(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO model_inference_requests (
               request_id,idempotency_key,request_hash,subject_type,subject_id,
               policy_id,policy_revision,policy_hash,purpose,purpose_hash,
               data_classification,provider_order_json,prompt_hash,context_hash,
               authority_hash,input_chars,max_output_tokens,state,selected_provider,
               selected_model,attempt_count,result_hash,created_at_utc,started_at_utc,
               completed_at_utc
             ) VALUES (
               ?1,'phase20-contract-idempotency','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
               'local_control_center','local_control_center',
               '00000000-0000-4000-8000-000000000020',1,
               '113e81b53bfb95b1d9496660cca07b44865aafdbf8c54e57076515600be10a51',
               'agent_workspace','6428a32c45677cf7ec4f6d2384fc81b5a62372106031a537bd4d313410e7d0c6',
               'private_derived','[\"ollama\"]',
               'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
               'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
               'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
               18,128,'completed','ollama','llama3.2:3b',1,
               'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
               '2026-07-31T12:00:00.000Z','2026-07-31T12:00:01.000Z','2026-07-31T12:00:02.000Z'
             )",
            params!["10000000-0000-4000-8000-000000000020"],
        )
        .expect("terminal request");
    connection
        .execute(
            "INSERT INTO model_inference_attempts (
               attempt_id,request_id,attempt_sequence,provider_key,model_id,
               authority_hash,decision_hash,state,output_hash,started_at_utc,completed_at_utc
             ) VALUES (
               '20000000-0000-4000-8000-000000000020',?1,1,'ollama','llama3.2:3b',
               'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
               'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff',
               'succeeded','eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
               '2026-07-31T12:00:01.000Z','2026-07-31T12:00:02.000Z'
             )",
            params!["10000000-0000-4000-8000-000000000020"],
        )
        .expect("terminal attempt");
    connection
        .execute(
            "INSERT INTO model_inference_private_results (
               request_id,classification,output_text,output_bytes,output_hash,created_at_utc
             ) VALUES (?1,'private','private result',14,
               'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
               '2026-07-31T12:00:02.000Z')",
            params!["10000000-0000-4000-8000-000000000020"],
        )
        .expect("private result");
    connection
        .execute(
            "INSERT INTO model_inference_receipts (
               receipt_id,request_id,subject_type,subject_id,policy_id,policy_revision,
               purpose_hash,data_classification,provider_key,model_id,outcome,result_code,
               request_hash,authority_hash,prompt_hash,context_hash,result_hash,
               receipt_hash,completed_at_utc
             ) VALUES (
               '30000000-0000-4000-8000-000000000020',?1,
               'local_control_center','local_control_center',
               '00000000-0000-4000-8000-000000000020',1,
               '6428a32c45677cf7ec4f6d2384fc81b5a62372106031a537bd4d313410e7d0c6',
               'private_derived','ollama','llama3.2:3b','completed','model_inference_completed',
               'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
               'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
               'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
               'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
               'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
               '9999999999999999999999999999999999999999999999999999999999999999',
               '2026-07-31T12:00:02.000Z'
             )",
            params!["10000000-0000-4000-8000-000000000020"],
        )
        .expect("receipt");
    connection
        .execute(
            "INSERT INTO model_inference_events (
               event_id,request_id,policy_id,event_type,outcome,actor_type,actor_id,
               detail_code,metadata_json,event_hash,created_at_utc
             ) VALUES (
               '40000000-0000-4000-8000-000000000020',?1,
               '00000000-0000-4000-8000-000000000020','model.inference_completed',
               'success','system','phase20-contract','private_result_retained','{}',
               '8888888888888888888888888888888888888888888888888888888888888888',
               '2026-07-31T12:00:02.000Z'
             )",
            params!["10000000-0000-4000-8000-000000000020"],
        )
        .expect("event");
}

#[test]
fn terminal_requests_attempts_results_receipts_and_events_are_immutable() {
    let connection = database();
    insert_terminal_fixture(&connection);
    for sql in [
        "UPDATE model_inference_requests SET result_hash=NULL WHERE request_id='10000000-0000-4000-8000-000000000020'",
        "DELETE FROM model_inference_requests WHERE request_id='10000000-0000-4000-8000-000000000020'",
        "UPDATE model_inference_attempts SET output_hash=NULL WHERE attempt_id='20000000-0000-4000-8000-000000000020'",
        "DELETE FROM model_inference_attempts WHERE attempt_id='20000000-0000-4000-8000-000000000020'",
        "UPDATE model_inference_private_results SET output_text='changed' WHERE request_id='10000000-0000-4000-8000-000000000020'",
        "DELETE FROM model_inference_private_results WHERE request_id='10000000-0000-4000-8000-000000000020'",
        "UPDATE model_inference_receipts SET result_code='changed' WHERE receipt_id='30000000-0000-4000-8000-000000000020'",
        "DELETE FROM model_inference_receipts WHERE receipt_id='30000000-0000-4000-8000-000000000020'",
        "UPDATE model_inference_events SET detail_code='changed' WHERE event_id='40000000-0000-4000-8000-000000000020'",
        "DELETE FROM model_inference_events WHERE event_id='40000000-0000-4000-8000-000000000020'",
    ] {
        assert!(connection.execute(sql, []).is_err(), "mutation unexpectedly succeeded: {sql}");
    }
}
