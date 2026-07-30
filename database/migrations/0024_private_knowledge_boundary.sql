-- Phase 16E: private knowledge access and result-egress enforcement.
-- Private source data remains on HomeServer. Wrappers receive only destination-specific,
-- policy-approved projections, safe provenance, and bounded receipts.

CREATE TABLE IF NOT EXISTS data_classification_catalog (
  class_key TEXT PRIMARY KEY,
  description TEXT NOT NULL CHECK (length(description) BETWEEN 1 AND 400),
  sensitivity_tier TEXT NOT NULL CHECK (sensitivity_tier IN ('public','low','medium','high','critical')),
  wrapper_egress_mode TEXT NOT NULL CHECK (wrapper_egress_mode IN ('never','conditional','named_connection','same_wrapper','minimal')),
  default_retention_days INTEGER NOT NULL CHECK (default_retention_days BETWEEN 0 AND 3650),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','deprecated','disabled')),
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS private_resource_catalog (
  resource_id TEXT PRIMARY KEY,
  resource_namespace TEXT NOT NULL CHECK (length(resource_namespace) BETWEEN 1 AND 80),
  resource_type TEXT NOT NULL CHECK (length(resource_type) BETWEEN 1 AND 80),
  local_source_id TEXT NOT NULL CHECK (length(local_source_id) BETWEEN 1 AND 240),
  local_display_name TEXT NOT NULL DEFAULT '' CHECK (length(local_display_name) <= 400),
  source_hash TEXT,
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','missing','quarantined','deleted')),
  resource_revision INTEGER NOT NULL DEFAULT 1 CHECK (resource_revision >= 1),
  deleted_at_utc TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  UNIQUE (resource_namespace, local_source_id)
);
CREATE INDEX IF NOT EXISTS idx_private_resources_state ON private_resource_catalog (resource_namespace,state,updated_at_utc DESC,resource_id);

CREATE TABLE IF NOT EXISTS private_resource_classifications (
  classification_id TEXT PRIMARY KEY,
  resource_id TEXT NOT NULL,
  class_key TEXT NOT NULL,
  classification_revision INTEGER NOT NULL CHECK (classification_revision >= 1),
  state TEXT NOT NULL CHECK (state IN ('active','superseded','revoked')),
  classified_by_user_id TEXT NOT NULL CHECK (length(classified_by_user_id) BETWEEN 1 AND 160),
  reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
  created_at_utc TEXT NOT NULL,
  revoked_at_utc TEXT,
  FOREIGN KEY (resource_id) REFERENCES private_resource_catalog(resource_id) ON DELETE CASCADE,
  FOREIGN KEY (class_key) REFERENCES data_classification_catalog(class_key) ON DELETE RESTRICT,
  UNIQUE (resource_id,classification_revision)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_private_resource_one_active_classification ON private_resource_classifications(resource_id) WHERE state='active';

CREATE TABLE IF NOT EXISTS private_resource_selectors (
  selector_id TEXT PRIMARY KEY,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  grant_id TEXT NOT NULL,
  grant_revision INTEGER NOT NULL CHECK (grant_revision >= 1),
  selector_revision INTEGER NOT NULL DEFAULT 1 CHECK (selector_revision >= 1),
  agent_id TEXT,
  agent_revision INTEGER,
  resource_namespace TEXT NOT NULL CHECK (length(resource_namespace) BETWEEN 1 AND 80),
  resource_type TEXT NOT NULL CHECK (length(resource_type) BETWEEN 1 AND 80),
  allowed_operations_json TEXT NOT NULL CHECK (json_valid(allowed_operations_json)),
  maximum_items INTEGER NOT NULL CHECK (maximum_items BETWEEN 1 AND 500),
  maximum_source_bytes INTEGER NOT NULL CHECK (maximum_source_bytes BETWEEN 1024 AND 104857600),
  purpose TEXT NOT NULL CHECK (length(purpose) BETWEEN 1 AND 1000),
  purpose_hash TEXT NOT NULL CHECK (length(purpose_hash)=64),
  output_schema TEXT NOT NULL CHECK (length(output_schema) BETWEEN 1 AND 160),
  allow_citations INTEGER NOT NULL DEFAULT 0 CHECK (allow_citations IN (0,1)),
  remote_model_mode TEXT NOT NULL DEFAULT 'disabled' CHECK (remote_model_mode IN ('disabled','local_only','approved_provider')),
  approved_remote_provider TEXT,
  egress_approval_mode TEXT NOT NULL DEFAULT 'preauthorized' CHECK (egress_approval_mode IN ('preauthorized','per_result')),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','suspended','expired','revoked')),
  created_by_user_id TEXT NOT NULL CHECK (length(created_by_user_id) BETWEEN 1 AND 160),
  reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
  expires_at_utc TEXT NOT NULL,
  revoked_at_utc TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE CASCADE,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_private_selectors_authorize ON private_resource_selectors(connection_id,grant_id,state,expires_at_utc,selector_id);

CREATE TABLE IF NOT EXISTS private_selector_resources (
  selector_id TEXT NOT NULL,
  resource_id TEXT NOT NULL,
  captured_resource_revision INTEGER NOT NULL CHECK (captured_resource_revision >= 1),
  captured_classification_revision INTEGER NOT NULL CHECK (captured_classification_revision >= 1),
  created_at_utc TEXT NOT NULL,
  PRIMARY KEY(selector_id,resource_id),
  FOREIGN KEY(selector_id) REFERENCES private_resource_selectors(selector_id) ON DELETE CASCADE,
  FOREIGN KEY(resource_id) REFERENCES private_resource_catalog(resource_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_private_selector_resources_resource ON private_selector_resources(resource_id,selector_id);

CREATE TABLE IF NOT EXISTS private_resource_aliases (
  alias_id TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL,
  resource_id TEXT NOT NULL,
  alias_reference TEXT NOT NULL CHECK (length(alias_reference) BETWEEN 12 AND 160),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','revoked')),
  created_at_utc TEXT NOT NULL,
  revoked_at_utc TEXT,
  FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY(resource_id) REFERENCES private_resource_catalog(resource_id) ON DELETE CASCADE,
  UNIQUE(connection_id,resource_id),
  UNIQUE(connection_id,alias_reference)
);

CREATE TABLE IF NOT EXISTS wrapper_job_privacy_bindings (
  job_id TEXT PRIMARY KEY,
  selector_id TEXT NOT NULL,
  selector_revision INTEGER NOT NULL CHECK(selector_revision >= 1),
  purpose_hash TEXT NOT NULL CHECK(length(purpose_hash)=64),
  output_schema TEXT NOT NULL,
  remote_model_provider TEXT,
  classification_set_hash TEXT NOT NULL CHECK(length(classification_set_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY(job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE,
  FOREIGN KEY(selector_id) REFERENCES private_resource_selectors(selector_id) ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS idx_wrapper_job_privacy_selector ON wrapper_job_privacy_bindings(selector_id,created_at_utc DESC,job_id);

CREATE TABLE IF NOT EXISTS private_knowledge_access_receipts (
  access_id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL,
  selector_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  operation TEXT NOT NULL CHECK(length(operation) BETWEEN 1 AND 80),
  query_hash TEXT NOT NULL CHECK(length(query_hash)=64),
  result_hash TEXT NOT NULL CHECK(length(result_hash)=64),
  source_count INTEGER NOT NULL CHECK(source_count BETWEEN 0 AND 500),
  source_bytes INTEGER NOT NULL CHECK(source_bytes BETWEEN 0 AND 104857600),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY(job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE,
  FOREIGN KEY(selector_id) REFERENCES private_resource_selectors(selector_id) ON DELETE RESTRICT,
  FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_private_access_receipts_job ON private_knowledge_access_receipts(job_id,created_at_utc DESC,access_id);

CREATE TABLE IF NOT EXISTS egress_decisions (
  decision_id TEXT PRIMARY KEY,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  job_id TEXT NOT NULL UNIQUE,
  selector_id TEXT,
  grant_id TEXT NOT NULL,
  grant_revision INTEGER NOT NULL CHECK(grant_revision >= 1),
  connection_authority_revision INTEGER NOT NULL CHECK(connection_authority_revision >= 0),
  output_schema TEXT NOT NULL CHECK(length(output_schema) BETWEEN 1 AND 160),
  input_classes_json TEXT NOT NULL CHECK(json_valid(input_classes_json)),
  output_classes_json TEXT NOT NULL CHECK(json_valid(output_classes_json)),
  policy TEXT NOT NULL CHECK(policy IN ('allow','allow_with_redaction','pending_review','deny')),
  state TEXT NOT NULL CHECK(state IN ('allowed','pending_review','denied','revoked')),
  detail_code TEXT NOT NULL CHECK(length(detail_code) BETWEEN 1 AND 120),
  approval_required INTEGER NOT NULL DEFAULT 0 CHECK(approval_required IN (0,1)),
  output_hash TEXT,
  private_evidence_hash TEXT NOT NULL CHECK(length(private_evidence_hash)=64),
  scan_version TEXT NOT NULL CHECK(length(scan_version) BETWEEN 1 AND 120),
  created_at_utc TEXT NOT NULL,
  decided_at_utc TEXT,
  delivered_at_utc TEXT,
  revoked_at_utc TEXT,
  FOREIGN KEY(wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY(job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE,
  FOREIGN KEY(selector_id) REFERENCES private_resource_selectors(selector_id) ON DELETE SET NULL,
  FOREIGN KEY(grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS idx_egress_decisions_connection ON egress_decisions(connection_id,state,created_at_utc DESC,decision_id);

CREATE TABLE IF NOT EXISTS wrapper_resource_projections (
  projection_id TEXT PRIMARY KEY,
  decision_id TEXT NOT NULL UNIQUE,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  job_id TEXT NOT NULL UNIQUE,
  selector_id TEXT,
  output_schema TEXT NOT NULL,
  safe_result_json TEXT NOT NULL CHECK(json_valid(safe_result_json)),
  output_hash TEXT NOT NULL CHECK(length(output_hash)=64),
  source_count INTEGER NOT NULL CHECK(source_count BETWEEN 0 AND 500),
  state TEXT NOT NULL CHECK(state IN ('active','pending_review','revoked','expired')),
  expires_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  revoked_at_utc TEXT,
  FOREIGN KEY(decision_id) REFERENCES egress_decisions(decision_id) ON DELETE CASCADE,
  FOREIGN KEY(wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY(job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE,
  FOREIGN KEY(selector_id) REFERENCES private_resource_selectors(selector_id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_wrapper_projections_connection ON wrapper_resource_projections(connection_id,state,expires_at_utc,projection_id);

CREATE TABLE IF NOT EXISTS egress_redactions (
  redaction_id TEXT PRIMARY KEY,
  decision_id TEXT NOT NULL,
  category TEXT NOT NULL CHECK(length(category) BETWEEN 1 AND 120),
  json_path_hash TEXT NOT NULL CHECK(length(json_path_hash)=64),
  match_hash TEXT NOT NULL CHECK(length(match_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY(decision_id) REFERENCES egress_decisions(decision_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_egress_redactions_decision ON egress_redactions(decision_id,category,redaction_id);

CREATE TABLE IF NOT EXISTS egress_approvals (
  approval_id TEXT PRIMARY KEY,
  decision_id TEXT NOT NULL UNIQUE,
  output_hash TEXT NOT NULL CHECK(length(output_hash)=64),
  state TEXT NOT NULL CHECK(state IN ('pending','approved','rejected','expired','revoked','consumed')),
  requested_by_user_id TEXT NOT NULL CHECK(length(requested_by_user_id) BETWEEN 1 AND 160),
  decided_by_user_id TEXT,
  reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 500),
  expires_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  decided_at_utc TEXT,
  consumed_at_utc TEXT,
  FOREIGN KEY(decision_id) REFERENCES egress_decisions(decision_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS private_evidence_records (
  evidence_id TEXT PRIMARY KEY,
  decision_id TEXT NOT NULL UNIQUE,
  job_id TEXT NOT NULL,
  evidence_hash TEXT NOT NULL CHECK(length(evidence_hash)=64),
  source_set_hash TEXT NOT NULL CHECK(length(source_set_hash)=64),
  private_result_hash TEXT NOT NULL CHECK(length(private_result_hash)=64),
  retention_until_utc TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active','expired','tombstone')),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY(decision_id) REFERENCES egress_decisions(decision_id) ON DELETE CASCADE,
  FOREIGN KEY(job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS privacy_boundary_incidents (
  incident_id TEXT PRIMARY KEY,
  wrapper_id TEXT,
  connection_id TEXT,
  job_id TEXT,
  selector_id TEXT,
  severity TEXT NOT NULL CHECK(severity IN ('low','medium','high','critical')),
  category TEXT NOT NULL CHECK(length(category) BETWEEN 1 AND 120),
  detail_code TEXT NOT NULL CHECK(length(detail_code) BETWEEN 1 AND 120),
  evidence_hash TEXT NOT NULL CHECK(length(evidence_hash)=64),
  state TEXT NOT NULL DEFAULT 'open' CHECK(state IN ('open','reviewed','resolved','dismissed')),
  detected_at_utc TEXT NOT NULL,
  resolved_at_utc TEXT,
  FOREIGN KEY(wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE SET NULL,
  FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE SET NULL,
  FOREIGN KEY(job_id) REFERENCES wrapper_jobs(job_id) ON DELETE SET NULL,
  FOREIGN KEY(selector_id) REFERENCES private_resource_selectors(selector_id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_privacy_incidents_queue ON privacy_boundary_incidents(severity,state,detected_at_utc DESC,incident_id);

CREATE TABLE IF NOT EXISTS projection_cache_entries (
  cache_id TEXT PRIMARY KEY,
  projection_id TEXT NOT NULL UNIQUE,
  connection_id TEXT NOT NULL,
  selector_id TEXT,
  source_revision_hash TEXT NOT NULL CHECK(length(source_revision_hash)=64),
  output_hash TEXT NOT NULL CHECK(length(output_hash)=64),
  state TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active','invalidated','expired')),
  expires_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  invalidated_at_utc TEXT,
  FOREIGN KEY(projection_id) REFERENCES wrapper_resource_projections(projection_id) ON DELETE CASCADE,
  FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY(selector_id) REFERENCES private_resource_selectors(selector_id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_projection_cache_expiration ON projection_cache_entries(connection_id,state,expires_at_utc,cache_id);

CREATE TABLE IF NOT EXISTS deletion_propagation_jobs (
  deletion_job_id TEXT PRIMARY KEY,
  resource_id TEXT NOT NULL,
  source_revision INTEGER NOT NULL CHECK(source_revision >= 1),
  state TEXT NOT NULL CHECK(state IN ('queued','running','completed','failed')),
  pending_targets_json TEXT NOT NULL CHECK(json_valid(pending_targets_json)),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK(attempt_count BETWEEN 0 AND 100),
  failure_code TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  FOREIGN KEY(resource_id) REFERENCES private_resource_catalog(resource_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_deletion_propagation_queue ON deletion_propagation_jobs(state,updated_at_utc,deletion_job_id);

INSERT OR IGNORE INTO data_classification_catalog(class_key,description,sensitivity_tier,wrapper_egress_mode,default_retention_days) VALUES
('secret','Credentials, keys, recovery material, and bearer authority.','critical','never',0),
('private_source','Raw files, messages, contacts, memories, source records, and local identifiers.','high','never',365),
('private_derived','Answers, summaries, classifications, and drafts derived from private sources.','high','conditional',90),
('private_selector','Local query, resource selector, and purpose authority.','high','never',90),
('shared_approved','Owner-approved data released to one named connection and purpose.','medium','named_connection',90),
('wrapper_owned','Input supplied by a wrapper and isolated to that same wrapper.','medium','same_wrapper',30),
('public','Owner-designated public material.','public','conditional',3650),
('safe_receipt','Bounded evidence without source content.','low','same_wrapper',365),
('security_metadata','Hashes, counts, categories, timestamps, and minimal security evidence.','medium','minimal',365);

INSERT OR IGNORE INTO private_resource_catalog(resource_id,resource_namespace,resource_type,local_source_id,local_display_name,source_hash,state,resource_revision,created_at_utc,updated_at_utc)
SELECT 'vault:'||document_id,'knowledge','document',document_id,title,sha256,CASE WHEN state='missing' THEN 'missing' WHEN state='failed' THEN 'quarantined' ELSE 'active' END,1,created_at_utc,updated_at_utc FROM vault_documents;
INSERT OR IGNORE INTO private_resource_classifications(classification_id,resource_id,class_key,classification_revision,state,classified_by_user_id,reason,created_at_utc)
SELECT 'vault-class:'||document_id,'vault:'||document_id,'private_source',1,'active','homeserver-system','Knowledge Vault default private-source classification',created_at_utc FROM vault_documents;

CREATE TRIGGER IF NOT EXISTS trg_privacy_vault_insert AFTER INSERT ON vault_documents BEGIN
  INSERT OR IGNORE INTO private_resource_catalog(resource_id,resource_namespace,resource_type,local_source_id,local_display_name,source_hash,state,resource_revision,created_at_utc,updated_at_utc)
  VALUES('vault:'||NEW.document_id,'knowledge','document',NEW.document_id,NEW.title,NEW.sha256,CASE WHEN NEW.state='missing' THEN 'missing' WHEN NEW.state='failed' THEN 'quarantined' ELSE 'active' END,1,NEW.created_at_utc,NEW.updated_at_utc);
  INSERT OR IGNORE INTO private_resource_classifications(classification_id,resource_id,class_key,classification_revision,state,classified_by_user_id,reason,created_at_utc)
  VALUES('vault-class:'||NEW.document_id,'vault:'||NEW.document_id,'private_source',1,'active','homeserver-system','Knowledge Vault default private-source classification',NEW.created_at_utc);
END;

CREATE TRIGGER IF NOT EXISTS trg_privacy_vault_update AFTER UPDATE OF state,sha256,title,updated_at_utc ON vault_documents BEGIN
  UPDATE private_resource_catalog SET local_display_name=NEW.title,source_hash=NEW.sha256,state=CASE WHEN NEW.state='missing' THEN 'missing' WHEN NEW.state='failed' THEN 'quarantined' ELSE 'active' END,resource_revision=resource_revision+CASE WHEN OLD.sha256<>NEW.sha256 OR OLD.state<>NEW.state THEN 1 ELSE 0 END,updated_at_utc=NEW.updated_at_utc WHERE resource_namespace='knowledge' AND local_source_id=NEW.document_id;
  UPDATE wrapper_resource_projections SET state='revoked',revoked_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE selector_id IN (SELECT selector_id FROM private_selector_resources WHERE resource_id='vault:'||NEW.document_id) AND state IN ('active','pending_review');
  UPDATE projection_cache_entries SET state='invalidated',invalidated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE selector_id IN (SELECT selector_id FROM private_selector_resources WHERE resource_id='vault:'||NEW.document_id) AND state='active';
END;

CREATE TRIGGER IF NOT EXISTS trg_privacy_vault_delete AFTER DELETE ON vault_documents BEGIN
  UPDATE private_resource_catalog SET state='deleted',resource_revision=resource_revision+1,deleted_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE resource_namespace='knowledge' AND local_source_id=OLD.document_id;
  INSERT OR IGNORE INTO deletion_propagation_jobs(deletion_job_id,resource_id,source_revision,state,pending_targets_json,attempt_count,created_at_utc,updated_at_utc)
  SELECT 'delete:'||resource_id||':'||resource_revision,resource_id,resource_revision,'queued','["selectors","projections","cache","deliveries"]',0,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now') FROM private_resource_catalog WHERE resource_namespace='knowledge' AND local_source_id=OLD.document_id;
END;

INSERT OR IGNORE INTO schema_migrations(migration_key) VALUES('0024_private_knowledge_boundary');
