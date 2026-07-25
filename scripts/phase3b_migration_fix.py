from pathlib import Path

store = Path("crates/homeserver-service/src/update_store.rs")
text = store.read_text(encoding="utf-8")
text = text.replace(
    '''pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(UPDATE_MIGRATION)?;
    health_check(connection)
}''',
    '''pub fn initialize(connection: &Connection) -> Result<()> {
    let applied: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![UPDATE_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    if applied == 0 {
        connection.execute_batch(UPDATE_MIGRATION)?;
    }
    health_check(connection)
}''',
    1,
)
text = text.replace(
    '''pub fn mark_applying(
    connection: &Connection,
    update_id: &str,
    rollback_path: &Path,
) -> Result<StoredUpdate> {
    connection.execute(
        "UPDATE update_records SET state='applying',rollback_path=?1,failure_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE update_id=?2 AND state='staged'",
        params![rollback_path.to_string_lossy(), update_id],
    )?;''',
    '''pub fn mark_applying(
    connection: &Connection,
    update_id: &str,
    rollback_path: &Path,
    pre_update_backup_id: &str,
) -> Result<StoredUpdate> {
    let changed = connection.execute(
        "UPDATE update_records SET state='applying',rollback_path=?1,pre_update_backup_id=?2,failure_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE update_id=?3 AND state='staged'",
        params![
            rollback_path.to_string_lossy(),
            pre_update_backup_id,
            update_id
        ],
    )?;
    ensure!(changed == 1, "staged update state changed before application began");''',
    1,
)
text = text.replace(
    '''        "{}",
    )?;
    update_by_id(connection, update_id)
}

pub fn mark_failure''',
    '''        &serde_json::json!({"pre_update_backup_id": pre_update_backup_id}).to_string(),
    )?;
    update_by_id(connection, update_id)
}

pub fn mark_failure''',
    1,
)
legacy_test = '''
    #[test]
    fn legacy_unsigned_update_rows_are_preserved_as_failed_audit_records() {
        let directory = tempdir().unwrap();
        let connection =
            database::initialize(&directory.path().join("homeserver.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO update_records (update_id,version,state,manifest_url,installer_sha256,manifest_signature) VALUES (?1,?2,'discovered',?3,?4,?5)",
                params![
                    "legacy-update",
                    "0.0.9",
                    "https://legacy.example/manifest.json",
                    "a".repeat(64),
                    "legacy-signature"
                ],
            )
            .unwrap();

        initialize(&connection).unwrap();
        let stored = update_by_id(&connection, "legacy-update").unwrap();
        assert_eq!(stored.record.state, UpdateState::Failed);
        assert_eq!(
            stored.record.failure_code.as_deref(),
            Some("legacy_unsigned_update_record")
        );
        assert_eq!(stored.manifest.key_id, "legacy-untrusted");
        initialize(&connection).unwrap();
        health_check(&connection).unwrap();
    }
'''
if "legacy_unsigned_update_rows_are_preserved_as_failed_audit_records" not in text:
    text = text.replace("\n    #[test]\n    fn available_update_round_trips", legacy_test + "\n    #[test]\n    fn available_update_round_trips", 1)
store.write_text(text, encoding="utf-8")

main = Path("crates/homeserver-service/src/main.rs")
text = main.read_text(encoding="utf-8")
old = '''        update_store::mark_applying(
            &*self.connection()?,
            &stored.record.update_id,
            &rollback_path,
        )?;'''
new = '''        update_store::mark_applying(
            &*self.connection()?,
            &stored.record.update_id,
            &rollback_path,
            &backup_result.backup.backup_id,
        )?;'''
if old not in text:
    raise RuntimeError("mark_applying call anchor was not found")
main.write_text(text.replace(old, new, 1), encoding="utf-8")
