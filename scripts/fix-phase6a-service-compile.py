#!/usr/bin/env python3
from pathlib import Path

path = Path(__file__).resolve().parents[1] / "crates/homeserver-service/src/microgifter_connection.rs"
text = path.read_text(encoding="utf-8")

replacements = [
    (
        "use rusqlite::{params, Connection, OptionalExtension, Row};",
        "use rusqlite::{params, Connection, OptionalExtension};",
    ),
    (
        '''    let connection_id = match request.connection_id {
        Some(value) => value,
        None => default_phase6a_connection_id(&*state.connection()?)?
            .context("paid update authorization requires an active Microgifter connection")?,
    };''',
        '''    let connection_id = match request.connection_id.as_deref() {
        Some(value) => value.to_owned(),
        None => default_phase6a_connection_id(&*state.connection()?)?
            .context("paid update authorization requires an active Microgifter connection")?,
    };''',
    ),
    (
        '''        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?''',
        '''        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        let pending = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        pending''',
    ),
    (
        '''    Ok(statement
        .query_map(params![connection_id], |row| {
            Ok(CapabilitySnapshot {
                capability_id: row.get(0)?,
                grant_state: row.get(1)?,
                source: row.get(2)?,
                expires_at_utc: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)''',
        '''    let rows = statement.query_map(params![connection_id], |row| {
        Ok(CapabilitySnapshot {
            capability_id: row.get(0)?,
            grant_state: row.get(1)?,
            source: row.get(2)?,
            expires_at_utc: row.get(3)?,
        })
    })?;
    let snapshots = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(snapshots)''',
    ),
    (
        '''    Ok(statement
        .query_map(params![limit.max(1).min(100) as i64], |row| {
            Ok(ReceiptSnapshot {
                receipt_id: row.get(0)?,
                event_type: row.get(1)?,
                result_category: row.get(2)?,
                error_category: row.get(3)?,
                previous_state: row.get(4)?,
                new_state: row.get(5)?,
                created_at_utc: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)''',
        '''    let rows = statement.query_map(params![limit.max(1).min(100) as i64], |row| {
        Ok(ReceiptSnapshot {
            receipt_id: row.get(0)?,
            event_type: row.get(1)?,
            result_category: row.get(2)?,
            error_category: row.get(3)?,
            previous_state: row.get(4)?,
            new_state: row.get(5)?,
            created_at_utc: row.get(6)?,
        })
    })?;
    let receipts = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(receipts)''',
    ),
    (
        '''    Ok(statement
        .query_map(params![PROVIDER_KEY], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)''',
        '''    let rows = statement.query_map(params![PROVIDER_KEY], |row| row.get::<_, String>(0))?;
    let connection_ids = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(connection_ids)''',
    ),
]

for old, new in replacements:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected exactly one Phase 6A compile repair anchor, found {count}: {old[:100]!r}")
    text = text.replace(old, new, 1)

path.write_text(text, encoding="utf-8", newline="\n")
print("Phase 6A service compiler defects repaired.")
