-- Phase 16C supplemental authority snapshot.
-- Captures the connection-wide authority fence independently from the individual
-- grant revision so queued and leased work can fail closed after revocation.

CREATE TABLE IF NOT EXISTS wrapper_job_authority_snapshots (
  job_id TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL,
  grant_id TEXT NOT NULL,
  connection_authority_revision INTEGER NOT NULL CHECK (connection_authority_revision >= 0),
  grant_revision INTEGER NOT NULL CHECK (grant_revision >= 1),
  authorization_decision_id TEXT NOT NULL,
  captured_at_utc TEXT NOT NULL,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_wrapper_job_authority_fence
  ON wrapper_job_authority_snapshots (
    connection_id, connection_authority_revision, grant_id, grant_revision
  );

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0022a_wrapper_job_authority_snapshots');
