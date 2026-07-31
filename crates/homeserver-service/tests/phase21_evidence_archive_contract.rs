use rusqlite::Connection;

const MIGRATION: &str =
    include_str!("../../../database/migrations/0029_tamper_evident_evidence_archive.sql");

fn database() -> Connection {
    let connection = Connection::open_in_memory().expect("database");
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE schema_migrations (
               migration_key TEXT PRIMARY KEY,
               applied_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
             );",
        )
        .expect("base schema");
    connection
        .execute_batch(MIGRATION)
        .expect("Phase 21 migration");
    connection
}

#[test]
fn default_policy_is_bounded_and_machine_local() {
    let connection = database();
    let policy: (i64, i64, i64, i64, i64, String) = connection
        .query_row(
            "SELECT enabled,interval_hours,max_records_per_archive,retention_count,max_package_bytes,policy_hash FROM evidence_archive_policies WHERE policy_revision=1",
            [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
        )
        .expect("default policy");
    assert_eq!(policy.0, 1);
    assert!((1..=720).contains(&policy.1));
    assert!((100..=50_000).contains(&policy.2));
    assert!((1..=365).contains(&policy.3));
    assert!((1_048_576..=268_435_456).contains(&policy.4));
    assert_eq!(
        policy.5,
        "21bc488664102f117d7c1296383962d611aa521127368d983cbff37732939b04"
    );
}

#[test]
fn archive_policy_members_exports_and_events_are_immutable() {
    let connection = database();
    connection
        .execute(
            "INSERT INTO evidence_archives (
               archive_id,idempotency_key,policy_id,policy_revision,state,
               previous_archive_hash,archive_sequence,record_count,table_count,
               records_sha256,chain_root_hash,manifest_sha256,package_sha256,
               package_size_bytes,file_name,storage_path,created_by_type,
               created_by_id,created_at_utc,completed_at_utc,verified_at_utc
             ) VALUES (
               '10000000-0000-4000-8000-000000000021','phase21-test-archive',
               '00000000-0000-4000-8000-000000000029',1,'verified',
               '0000000000000000000000000000000000000000000000000000000000000000',
               1,1,1,
               'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
               'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
               'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
               'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
               1024,'evidence.mgha','C:/ProgramData/Microgifter/HomeServer/evidence-archives/evidence.mgha',
               'system','phase21-test','2026-07-31T12:00:00.000Z',
               '2026-07-31T12:00:01.000Z','2026-07-31T12:00:01.000Z'
             )",
            [],
        )
        .expect("archive");
    connection
        .execute(
            "INSERT INTO evidence_archive_storage (archive_id,state,last_verified_at_utc,updated_at_utc) VALUES ('10000000-0000-4000-8000-000000000021','present','2026-07-31T12:00:01.000Z','2026-07-31T12:00:01.000Z')",
            [],
        )
        .expect("storage");
    connection
        .execute(
            "INSERT INTO evidence_archive_members (member_id,archive_id,ordinal,source_table,source_key,record_sha256,chain_hash,created_at_utc) VALUES ('20000000-0000-4000-8000-000000000021','10000000-0000-4000-8000-000000000021',1,'agent_runtime_receipts','receipt-1','eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','2026-07-31T12:00:01.000Z')",
            [],
        )
        .expect("member");
    connection
        .execute(
            "INSERT INTO evidence_archive_exports (export_id,archive_id,package_sha256,destination_file_name,exported_by_user_id,export_receipt_hash,created_at_utc) VALUES ('30000000-0000-4000-8000-000000000021','10000000-0000-4000-8000-000000000021','dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd','evidence.mgha','local_control_center','ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff','2026-07-31T12:01:00.000Z')",
            [],
        )
        .expect("export");
    connection
        .execute(
            "INSERT INTO evidence_archive_events (event_id,archive_id,policy_id,event_type,outcome,actor_type,actor_id,detail_code,event_hash,created_at_utc) VALUES ('40000000-0000-4000-8000-000000000021','10000000-0000-4000-8000-000000000021','00000000-0000-4000-8000-000000000029','evidence.archive_verified','success','system','phase21-test','verified','abababababababababababababababababababababababababababababababab','2026-07-31T12:00:01.000Z')",
            [],
        )
        .expect("event");

    for sql in [
        "UPDATE evidence_archive_policies SET interval_hours=2 WHERE policy_revision=1",
        "DELETE FROM evidence_archive_policies WHERE policy_revision=1",
        "UPDATE evidence_archives SET package_sha256='eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee' WHERE archive_sequence=1",
        "UPDATE evidence_archives SET previous_archive_hash='eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee' WHERE archive_sequence=1",
        "UPDATE evidence_archives SET archive_sequence=2 WHERE archive_sequence=1",
        "UPDATE evidence_archives SET storage_path='C:/other.mgha' WHERE archive_sequence=1",
        "DELETE FROM evidence_archives WHERE archive_sequence=1",
        "UPDATE evidence_archive_members SET source_key='receipt-2' WHERE ordinal=1",
        "DELETE FROM evidence_archive_members WHERE ordinal=1",
        "UPDATE evidence_archive_exports SET destination_file_name='other.mgha' WHERE export_id='30000000-0000-4000-8000-000000000021'",
        "DELETE FROM evidence_archive_exports WHERE export_id='30000000-0000-4000-8000-000000000021'",
        "UPDATE evidence_archive_events SET detail_code='changed' WHERE event_id='40000000-0000-4000-8000-000000000021'",
        "DELETE FROM evidence_archive_events WHERE event_id='40000000-0000-4000-8000-000000000021'",
    ] {
        assert!(
            connection.execute(sql, []).is_err(),
            "immutable evidence mutation unexpectedly succeeded: {sql}"
        );
    }
}

#[test]
fn storage_retention_requires_exported_state_and_cannot_reverse_pruning() {
    let connection = database();
    connection
        .execute(
            "INSERT INTO evidence_archives (archive_id,idempotency_key,policy_id,policy_revision,state,previous_archive_hash,archive_sequence,file_name,storage_path,created_by_type,created_by_id,created_at_utc) VALUES ('50000000-0000-4000-8000-000000000021','phase21-storage-test','00000000-0000-4000-8000-000000000029',1,'failed','0000000000000000000000000000000000000000000000000000000000000000',1,'failed.mgha','C:/failed.mgha','system','phase21-test','2026-07-31T12:00:00.000Z')",
            [],
        )
        .expect("failed archive");
    connection
        .execute(
            "INSERT INTO evidence_archive_storage (archive_id,state,updated_at_utc) VALUES ('50000000-0000-4000-8000-000000000021','creating','2026-07-31T12:00:00.000Z')",
            [],
        )
        .expect("storage");
    connection
        .execute(
            "UPDATE evidence_archive_storage SET state='missing',updated_at_utc='2026-07-31T12:01:00.000Z' WHERE archive_id='50000000-0000-4000-8000-000000000021'",
            [],
        )
        .expect("creating to missing");
    assert!(connection
        .execute(
            "UPDATE evidence_archive_storage SET state='present' WHERE archive_id='50000000-0000-4000-8000-000000000021'",
            [],
        )
        .is_err());
}

#[test]
fn migration_registers_archive_contract_once() {
    let connection = database();
    connection
        .execute_batch(MIGRATION)
        .expect("idempotent migration");
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE migration_key='0029_tamper_evident_evidence_archive'",
            [],
            |row| row.get(0),
        )
        .expect("migration count");
    assert_eq!(count, 1);
}