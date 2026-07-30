pub fn reconcile_authority(connection: &Connection) -> Result<()> {
    let sql = format!(
        "{JOB_RECORD_SELECT} WHERE j.state IN ('queued','leased','running','waiting') ORDER BY j.created_at_utc,j.job_id"
    );
    let mut statement = connection.prepare(&sql)?;
    let jobs = statement
        .query_map([], job_record_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    if jobs.is_empty() {
        return Ok(());
    }
    let transaction = connection.unchecked_transaction()?;
    for job in jobs {
        let now = Utc::now();
        if parse_utc(&job.expires_at_utc, "job expiration")? <= now {
            expire_job_tx(&transaction, &job, "job_expired")?;
            continue;
        }
        if !authority_is_current_tx(&transaction, &job)? {
            cancel_for_authority_tx(&transaction, &job, "authority_changed")?;
            continue;
        }
        if matches!(job.state.as_str(), "leased" | "running") {
            let lease_expired = job
                .lease_expires_at_utc
                .as_deref()
                .map(|value| parse_utc(value, "lease expiration"))
                .transpose()?
                .is_some_and(|expiration| expiration <= now);
            if lease_expired {
                recover_expired_lease_tx(&transaction, &job)?;
            }
        }
    }
    transaction.commit()?;
    Ok(())
}

fn authority_is_current_tx(transaction: &Transaction<'_>, job: &JobRecord) -> Result<bool> {
    let context: Option<(String, i64, String, String, i64, String, String)> = transaction
        .query_row(
            "SELECT c.lifecycle_state,c.grant_revision,w.state,g.state,g.grant_revision,g.expires_at_utc,r.outcome FROM wrapper_connections c JOIN wrapper_identities w ON w.wrapper_id=c.wrapper_id JOIN wrapper_capability_grants g ON g.grant_id=?1 AND g.connection_id=c.connection_id AND g.wrapper_id=c.wrapper_id JOIN wrapper_authorization_receipts r ON r.decision_id=?2 AND r.connection_id=c.connection_id AND r.grant_id=g.grant_id WHERE c.connection_id=?3",
            params![job.grant_id, job.authorization_decision_id, job.connection_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
        )
        .optional()?;
    let Some((connection_state, connection_revision, wrapper_state, grant_state, grant_revision, grant_expires, decision_outcome)) = context else {
        return Ok(false);
    };
    Ok(matches!(connection_state.as_str(), "active" | "offline" | "grace")
        && wrapper_state == "active"
        && grant_state == "active"
        && decision_outcome == "allowed"
        && connection_revision.max(0) as u64 == job.connection_authority_revision
        && grant_revision.max(0) as u64 == job.grant_revision
        && parse_utc(&grant_expires, "grant expiration")? > Utc::now())
}

fn cancel_for_authority_tx(
    transaction: &Transaction<'_>,
    job: &JobRecord,
    detail_code: &str,
) -> Result<()> {
    if TERMINAL_STATES.contains(&job.state.as_str()) {
        return Ok(());
    }
    let now = now_utc();
    transaction.execute(
        "UPDATE wrapper_jobs SET state='cancelled',cancelled_at_utc=?1,completed_at_utc=?1,lease_owner_id=NULL,lease_token_hash=NULL,lease_expires_at_utc=NULL,failure_code=?2,updated_at_utc=?1 WHERE job_id=?3 AND state IN ('queued','leased','running','waiting')",
        params![now, detail_code, job.job_id],
    )?;
    let cancelled = job_record_by_id_tx(transaction, &job.job_id)?;
    record_job_event(
        transaction,
        &cancelled,
        JobEventEvidence {
            event_type: "wrapper.job.authority_cancelled",
            previous_state: Some(&job.state),
            current_state: "cancelled",
            outcome: "denied",
            detail_code,
            actor_type: "system",
            actor_id: "homeserver-authority-reconciler",
            visibility: "wrapper",
            metadata: json!({
                "grant_id": job.grant_id,
                "captured_grant_revision": job.grant_revision,
                "captured_connection_authority_revision": job.connection_authority_revision,
                "private_input_exposed": false
            }),
        },
    )?;
    create_terminal_receipt_tx(
        transaction,
        &cancelled,
        "cancelled",
        detail_code,
        None,
        None,
        job.lease_owner_id.as_deref(),
    )?;
    Ok(())
}

fn expire_job_tx(transaction: &Transaction<'_>, job: &JobRecord, detail_code: &str) -> Result<()> {
    if TERMINAL_STATES.contains(&job.state.as_str()) {
        return Ok(());
    }
    let now = now_utc();
    transaction.execute(
        "UPDATE wrapper_jobs SET state='expired',completed_at_utc=?1,lease_owner_id=NULL,lease_token_hash=NULL,lease_expires_at_utc=NULL,failure_code=?2,updated_at_utc=?1 WHERE job_id=?3 AND state IN ('queued','leased','running','waiting')",
        params![now, detail_code, job.job_id],
    )?;
    let expired = job_record_by_id_tx(transaction, &job.job_id)?;
    record_job_event(
        transaction,
        &expired,
        JobEventEvidence {
            event_type: "wrapper.job.expired",
            previous_state: Some(&job.state),
            current_state: "expired",
            outcome: "warning",
            detail_code,
            actor_type: "system",
            actor_id: "homeserver-job-scheduler",
            visibility: "wrapper",
            metadata: json!({"expires_at_utc": job.expires_at_utc}),
        },
    )?;
    create_terminal_receipt_tx(
        transaction,
        &expired,
        "expired",
        detail_code,
        None,
        None,
        job.lease_owner_id.as_deref(),
    )?;
    Ok(())
}

fn recover_expired_lease_tx(transaction: &Transaction<'_>, job: &JobRecord) -> Result<()> {
    let retryable = job.attempt_count < job.max_attempts
        && parse_utc(&job.expires_at_utc, "job expiration")? > Utc::now() + Duration::seconds(30);
    if retryable {
        let previous_state = job.state.clone();
        let available_at = (Utc::now() + Duration::seconds(15))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        transaction.execute(
            "UPDATE wrapper_jobs SET state='queued',available_at_utc=?1,lease_owner_id=NULL,lease_token_hash=NULL,lease_expires_at_utc=NULL,failure_code='lease_expired',updated_at_utc=?2 WHERE job_id=?3",
            params![available_at, now_utc(), job.job_id],
        )?;
        let queued = job_record_by_id_tx(transaction, &job.job_id)?;
        record_job_event(
            transaction,
            &queued,
            JobEventEvidence {
                event_type: "wrapper.job.lease_recovered",
                previous_state: Some(&previous_state),
                current_state: "queued",
                outcome: "warning",
                detail_code: "lease_expired_retry",
                actor_type: "system",
                actor_id: "homeserver-job-scheduler",
                visibility: "wrapper",
                metadata: json!({"attempt_count": queued.attempt_count}),
            },
        )?;
    } else {
        let previous_state = job.state.clone();
        let now = now_utc();
        transaction.execute(
            "UPDATE wrapper_jobs SET state='dead_letter',completed_at_utc=?1,lease_owner_id=NULL,lease_token_hash=NULL,lease_expires_at_utc=NULL,failure_code='lease_expired',updated_at_utc=?1 WHERE job_id=?2",
            params![now, job.job_id],
        )?;
        let terminal = job_record_by_id_tx(transaction, &job.job_id)?;
        record_job_event(
            transaction,
            &terminal,
            JobEventEvidence {
                event_type: "wrapper.job.dead_lettered",
                previous_state: Some(&previous_state),
                current_state: "dead_letter",
                outcome: "error",
                detail_code: "lease_expired_attempts_exhausted",
                actor_type: "system",
                actor_id: "homeserver-job-scheduler",
                visibility: "wrapper",
                metadata: json!({
                    "attempt_count": terminal.attempt_count,
                    "max_attempts": terminal.max_attempts
                }),
            },
        )?;
        create_terminal_receipt_tx(
            transaction,
            &terminal,
            "dead_letter",
            "lease_expired_attempts_exhausted",
            None,
            None,
            job.lease_owner_id.as_deref(),
        )?;
    }
    Ok(())
}
