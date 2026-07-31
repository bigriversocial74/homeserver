#!/usr/bin/env python3
from pathlib import Path
import json

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, text: str) -> None:
    (ROOT / path).write_text(text, encoding="utf-8")


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, found {count}")
    return text.replace(old, new, 1)


# Complete the Phase 21 storage-state and export-gated retention schema.
path = "database/migrations/0029_tamper_evident_evidence_archive.sql"
text = read(path)
storage_schema = """CREATE TABLE IF NOT EXISTS evidence_archive_storage (
  archive_id TEXT PRIMARY KEY,
  state TEXT NOT NULL CHECK (state IN ('creating','present','exported','pruned','missing')),
  last_verified_at_utc TEXT,
  exported_at_utc TEXT,
  pruned_at_utc TEXT,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (archive_id) REFERENCES evidence_archives(archive_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_evidence_archive_storage_state
  ON evidence_archive_storage (state,updated_at_utc DESC,archive_id DESC);

CREATE TRIGGER IF NOT EXISTS trg_evidence_archive_storage_transition_guard
BEFORE UPDATE ON evidence_archive_storage
WHEN NOT (
  (OLD.state='creating' AND NEW.state IN ('creating','present','missing')) OR
  (OLD.state='present' AND NEW.state IN ('present','exported','missing')) OR
  (OLD.state='exported' AND NEW.state IN ('exported','pruned','missing')) OR
  (OLD.state='pruned' AND NEW.state='pruned') OR
  (OLD.state='missing' AND NEW.state='missing')
)
BEGIN
  SELECT RAISE(ABORT,'evidence archive storage transition is invalid');
END;

CREATE TRIGGER IF NOT EXISTS trg_evidence_archive_storage_no_delete
BEFORE DELETE ON evidence_archive_storage
BEGIN
  SELECT RAISE(ABORT,'evidence archive storage history is retained');
END;

"""
text = replace_once(
    text,
    "CREATE TABLE IF NOT EXISTS evidence_archive_members (\n",
    storage_schema + "CREATE TABLE IF NOT EXISTS evidence_archive_members (\n",
    "archive storage schema",
)
# The validator requires this literal marker to prove why pruning is allowed.
text = text.replace(
    "-- Source evidence remains immutable and is not deleted by this phase.\n",
    "-- Source evidence remains immutable and is not deleted by this phase.\n-- Local package pruning is allowed only after export_verified_before_prune evidence.\n",
    1,
)
write(path, text)


# Repair and harden the staged service before integration.
path = "crates/homeserver-service/src/evidence_archive.rs"
text = read(path)
text = text.replace("use anyhow::{bail, ensure, Context, Result};", "use anyhow::{ensure, Context, Result};")
text = text.replace("    response::{IntoResponse, Response},", "    response::Response,")

old = """    let connection = state.connection()?;
    let package = package_for_export(state, &archive_id)?;
    let verified = verify_package_file(&connection, &state.config, &package.path, Some(&archive_id))?;
"""
new = """    let package = package_for_export(state, &archive_id)?;
    let connection = state.connection()?;
    let verified = verify_package_file(&connection, &state.config, &package.path, Some(&archive_id))?;
    ensure!(verified.size_bytes == package.size_bytes, "evidence archive package size is not the recorded size");
"""
text = replace_once(text, old, new, "verification lock repair")

old = """        let verified = verify_package_file(&connection, &state.config, &final_path, Some(&archive_id))?;
        ensure!(verified.package_sha256 == built.1, "evidence archive package hash changed after write");
        Ok((built, verified))
"""
new = """        let verified = verify_package_file(&connection, &state.config, &final_path, Some(&archive_id))?;
        ensure!(verified.package_sha256 == built.1, "evidence archive package hash changed after write");
        ensure!(verified.size_bytes == built.0.len() as u64, "evidence archive package size changed after write");
        Ok((built, verified))
"""
text = replace_once(text, old, new, "post-write size verification")

old = """    let canonical_path = path.canonicalize().or_else(|_| {
        let parent = path.parent().context("evidence archive path has no parent")?.canonicalize()?;
        let file_name = path.file_name().context("evidence archive path has no file name")?;
        Ok::<PathBuf, std::io::Error>(parent.join(file_name))
    })?;
"""
new = """    let canonical_path = if path.exists() {
        path.canonicalize()?
    } else {
        let parent = path
            .parent()
            .context("evidence archive path has no parent")?
            .canonicalize()?;
        let file_name = path
            .file_name()
            .context("evidence archive path has no file name")?;
        parent.join(file_name)
    };
"""
text = replace_once(text, old, new, "managed path repair")

old = """    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(5)?,
        ))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
"""
new = """    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(5)?,
        ))
    })?;
    let columns = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(columns)
"""
text = replace_once(text, old, new, "owned table columns")

old = """fn latest_verified_archive(connection: &Connection) -> Result<Option<(String, String)>> {
    connection
        .query_row(
            "SELECT archive_id,manifest_sha256 FROM evidence_archives WHERE state='verified' ORDER BY archive_sequence DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?,row.get(1)?)),
        )
        .optional()
        .map_err(Into::into)
}
"""
new = """fn latest_verified_archive(connection: &Connection) -> Result<Option<(String, String)>> {
    let archive = connection
        .query_row(
            "SELECT archive_id,manifest_sha256 FROM evidence_archives WHERE state='verified' ORDER BY archive_sequence DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?,row.get(1)?)),
        )
        .optional()?;
    Ok(archive)
}
"""
text = replace_once(text, old, new, "owned latest archive")

old = """    let mut statement = connection.prepare(
        "SELECT a.archive_id,a.storage_path FROM evidence_archives a JOIN evidence_archive_storage s ON s.archive_id=a.archive_id WHERE a.state='verified' AND s.state='exported' AND EXISTS (SELECT 1 FROM evidence_archive_exports x WHERE x.archive_id=a.archive_id) ORDER BY a.archive_sequence DESC LIMIT -1 OFFSET ?1",
    )?;
    let candidates = statement
        .query_map(params![policy.retention_count as i64], |row| {
"""
new = """    let mut statement = connection.prepare(
        "SELECT a.archive_id,a.storage_path FROM evidence_archives a JOIN evidence_archive_storage s ON s.archive_id=a.archive_id WHERE a.state='verified' AND s.state='exported' AND EXISTS (SELECT 1 FROM evidence_archive_exports x WHERE x.archive_id=a.archive_id) AND a.archive_id NOT IN (SELECT retained.archive_id FROM evidence_archives retained JOIN evidence_archive_storage retained_storage ON retained_storage.archive_id=retained.archive_id WHERE retained.state='verified' AND retained_storage.state IN ('present','exported') ORDER BY retained.archive_sequence DESC LIMIT ?1) ORDER BY a.archive_sequence ASC",
    )?;
    let candidates = statement
        .query_map(params![policy.retention_count as i64], |row| {
"""
text = replace_once(text, old, new, "retention candidate boundary")

# Borrow values inside hash documents so they remain available for inserts.
text = text.replace('"event_id": event_id,', '"event_id": &event_id,')
text = text.replace('"created_at_utc": created_at\n    }))?;', '"created_at_utc": &created_at\n    }))?;')
text = text.replace('"created_by_user_id": actor,\n        "reason": reason', '"created_by_user_id": &actor,\n        "reason": &reason')
text = text.replace('"export_id": export_id,\n        "archive_id": archive_id,\n        "package_sha256": package_hash,\n        "destination_file_name": destination,\n        "exported_by_user_id": actor,\n        "created_at_utc": now', '"export_id": &export_id,\n        "archive_id": &archive_id,\n        "package_sha256": &package_hash,\n        "destination_file_name": &destination,\n        "exported_by_user_id": &actor,\n        "created_at_utc": &now')
write(path, text)


# Register the service and lifecycle.
path = "crates/homeserver-service/src/main.rs"
text = read(path)
text = replace_once(text, "mod document_extraction;\n", "mod document_extraction;\nmod evidence_archive;\n", "service module registration")
old = """        if let Err(error) = inference_governance::health_check(&connection) {
            error!(
                ?error,
                "HomeServer model inference governance database health check failed"
            );
            return HealthSnapshot::needs_attention(
                &self.config.server_name,
                "model_inference_governance_integrity_check_failed",
            );
        }

        if let Err(error) = semantic_vault::health_check(&connection) {
"""
new = """        if let Err(error) = inference_governance::health_check(&connection) {
            error!(
                ?error,
                "HomeServer model inference governance database health check failed"
            );
            return HealthSnapshot::needs_attention(
                &self.config.server_name,
                "model_inference_governance_integrity_check_failed",
            );
        }

        if let Err(error) = evidence_archive::health_check(&connection, &self.config) {
            error!(?error, "HomeServer evidence archive health check failed");
            return HealthSnapshot::needs_attention(
                &self.config.server_name,
                "evidence_archive_integrity_check_failed",
            );
        }

        if let Err(error) = semantic_vault::health_check(&connection) {
"""
text = replace_once(text, old, new, "health integration")
write(path, text)

path = "crates/homeserver-service/src/app.rs"
text = read(path)
text = replace_once(
    text,
    "    inference_governance, knowledge_vault, mcp_runtime, microgifter_connection, model_center,\n",
    "    evidence_archive, inference_governance, knowledge_vault, mcp_runtime, microgifter_connection, model_center,\n",
    "app import",
)
text = replace_once(
    text,
    "    inference_governance::initialize(&connection)?;\n    semantic_vault::initialize(&connection)?;\n",
    "    inference_governance::initialize(&connection)?;\n    evidence_archive::initialize(&connection, &config)?;\n    semantic_vault::initialize(&connection)?;\n",
    "app initialize",
)
text = replace_once(
    text,
    "            .merge(inference_governance::router(state.clone()))\n            .merge(semantic_vault::router(state.clone()))\n",
    "            .merge(inference_governance::router(state.clone()))\n            .merge(evidence_archive::router(state.clone()))\n            .merge(semantic_vault::router(state.clone()))\n",
    "app router",
)
text = replace_once(
    text,
    "                        if let Err(error) = inference_governance::maintain_history(&connection) {\n                            warn!(?error, \"scheduled model inference retention failed\");\n                        }\n",
    "                        if let Err(error) = inference_governance::maintain_history(&connection) {\n                            warn!(?error, \"scheduled model inference retention failed\");\n                        }\n",
    "scheduler stable anchor",
)
old = """                    scheduled_state.create_automatic_backup_if_due()
"""
new = """                    if let Err(error) = evidence_archive::create_automatic_if_due(scheduled_state.clone()) {
                        warn!(?error, "scheduled evidence archive failed");
                    }
                    scheduled_state.create_automatic_backup_if_due()
"""
text = replace_once(text, old, new, "automatic archive scheduler")
write(path, text)


# Trusted Control Center API commands.
path = "src-tauri/src/runtime.rs"
text = read(path)
append = """

#[tauri::command]
pub(crate) async fn homeserver_evidence_archives() -> Result<Value, String> {
    get_json("/v1/evidence-archives").await
}

#[tauri::command]
pub(crate) async fn homeserver_update_evidence_archive_policy(policy: Value) -> Result<Value, String> {
    let mut object = policy
        .as_object()
        .cloned()
        .ok_or_else(|| "Evidence archive policy must be an object.".to_owned())?;
    object.insert(
        "created_by_user_id".to_owned(),
        Value::String(LOCAL_CONTROL_CENTER_ACTOR.to_owned()),
    );
    object.insert(
        "confirmation".to_owned(),
        Value::String("UPDATE EVIDENCE ARCHIVE POLICY".to_owned()),
    );
    post_json("/v1/evidence-archives/policies", &Value::Object(object)).await
}

#[tauri::command]
pub(crate) async fn homeserver_create_evidence_archive() -> Result<Value, String> {
    post_json(
        "/v1/evidence-archives/create",
        &json!({
            "idempotency_key": format!("control-center:{}", uuid::Uuid::new_v4()),
            "actor_user_id": LOCAL_CONTROL_CENTER_ACTOR,
            "confirmation": "CREATE EVIDENCE ARCHIVE"
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_verify_evidence_archive(
    archive_id: String,
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/evidence-archives/verify",
        &json!({
            "archive_id": archive_id,
            "actor_user_id": LOCAL_CONTROL_CENTER_ACTOR,
            "confirmation": confirmation
        }),
    )
    .await
}
"""
text += append
write(path, text)


# Bounded trusted desktop export and command registration.
path = "src-tauri/src/lib.rs"
text = read(path)
text = replace_once(
    text,
    "const MAX_RECOVERY_PACKAGE_BYTES: u64 = 320 * 1024 * 1024;\n",
    "const MAX_RECOVERY_PACKAGE_BYTES: u64 = 320 * 1024 * 1024;\nconst MAX_EVIDENCE_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;\n",
    "desktop archive size limit",
)
old = """fn safe_file_name(value: &str) -> String {
"""
new = """fn safe_evidence_archive_file_name(value: &str) -> String {
    let name: String = value
        .chars()
        .take(200)
        .map(|character| {
            if character.is_ascii_alphanumeric() || ".-_".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect();
    if name.ends_with(".mgha") && name.len() > ".mgha".len() {
        name
    } else {
        "Microgifter-HomeServer-Evidence.mgha".to_owned()
    }
}

fn safe_file_name(value: &str) -> String {
"""
text = replace_once(text, old, new, "safe archive filename")
anchor = """#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
"""
export_command = """#[tauri::command]
async fn homeserver_export_evidence_archive(
    archive_id: String,
    suggested_file_name: String,
    package_sha256: String,
) -> Result<Option<serde_json::Value>, String> {
    let file_name = safe_evidence_archive_file_name(&suggested_file_name);
    if package_sha256.len() != 64 || !package_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Evidence archive package hash is invalid.".to_owned());
    }
    let Some(destination) = AsyncFileDialog::new()
        .add_filter("Microgifter evidence archive", &["mgha"])
        .set_file_name(&file_name)
        .save_file()
        .await
    else {
        return Ok(None);
    };

    let response = client()?
        .get(format!(
            "{}/v1/evidence-archives/{}/package",
            api_base_url(),
            archive_id
        ))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return decode_json::<serde_json::Value>(response)
            .await
            .map(|_| None);
    }
    if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        != Some("application/vnd.microgifter.homeserver-evidence-archive")
    {
        return Err("HomeServer returned an unexpected evidence archive content type.".to_owned());
    }

    let destination_path = destination.path().to_path_buf();
    let mut output = tokio::fs::File::create(&destination_path)
        .await
        .map_err(|error| error.to_string())?;
    let mut stream = response.bytes_stream();
    let mut total_bytes = 0_u64;
    let transfer_result = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| error.to_string())?;
            total_bytes = total_bytes
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| "Evidence archive export size overflow.".to_owned())?;
            if total_bytes > MAX_EVIDENCE_ARCHIVE_BYTES {
                return Err("Evidence archive export exceeds the package size limit.".to_owned());
            }
            output
                .write_all(&chunk)
                .await
                .map_err(|error| error.to_string())?;
        }
        if total_bytes <= 12 {
            return Err("Evidence archive export was empty or truncated.".to_owned());
        }
        output.sync_all().await.map_err(|error| error.to_string())?;
        Ok::<(), String>(())
    }
    .await;
    if let Err(error) = transfer_result {
        drop(output);
        let _ = tokio::fs::remove_file(&destination_path).await;
        return Err(error);
    }

    let receipt: serde_json::Value = post_json(
        "/v1/evidence-archives/exports",
        &serde_json::json!({
            "archive_id": archive_id,
            "package_sha256": package_sha256,
            "destination_file_name": file_name,
            "actor_user_id": "local_control_center",
            "confirmation": format!("EXPORT EVIDENCE ARCHIVE {}", archive_id)
        }),
    )
    .await?;
    Ok(Some(serde_json::json!({
        "path": destination_path.to_string_lossy(),
        "receipt": receipt
    })))
}

"""
text = replace_once(text, anchor, export_command + anchor, "desktop export command")
text = replace_once(
    text,
    "            runtime::homeserver_cancel_model_inference,\n",
    "            runtime::homeserver_cancel_model_inference,\n            runtime::homeserver_evidence_archives,\n            runtime::homeserver_update_evidence_archive_policy,\n            runtime::homeserver_create_evidence_archive,\n            runtime::homeserver_verify_evidence_archive,\n",
    "runtime command registration",
)
text = replace_once(
    text,
    "            homeserver_export_recovery_package,\n",
    "            homeserver_export_recovery_package,\n            homeserver_export_evidence_archive,\n",
    "archive export registration",
)
write(path, text)


# Agent Runtime Control Center archive status and actions.
path = "src/agent-runtime-control-center.js"
text = read(path)
text = replace_once(text, "  governance: null,\n", "  governance: null,\n  archives: null,\n", "archive state")
render_function = r'''
function renderEvidenceArchives() {
  const snapshot = runtimeState.archives || {};
  const policy = snapshot.policy || {};
  const archives = values(snapshot.archives);
  const boundarySafe = snapshot.private_content_exposed === false
    && snapshot.source_evidence_deleted === false;
  return `<section class="panel runtime-evidence-archive ${boundarySafe ? "safe" : "unsafe"}">
    <div class="panel-title"><div>${icon("logs", 18)}<div><h2>Tamper-evident evidence archives</h2><p>Machine-encrypted, hash-chained receipts and events with independently verifiable export.</p></div></div><div class="runtime-archive-actions"><span>${Number(snapshot.unarchived_record_count || 0)} unarchived</span><button class="button secondary" type="button" data-evidence-policy-update>Policy</button><button class="button primary" type="button" data-evidence-archive-create>Create archive</button></div></div>
    <div class="runtime-safety-grid">
      ${safetyItem("Private content", snapshot.private_content_exposed === false ? "Excluded" : "Exposure detected", snapshot.private_content_exposed === false)}
      ${safetyItem("Source evidence", snapshot.source_evidence_deleted === false ? "Never deleted" : "Deletion detected", snapshot.source_evidence_deleted === false)}
      ${safetyItem("Policy", policy.enabled ? `Revision ${Number(policy.policy_revision || 0)} active` : "Automatic archive disabled", Boolean(policy.policy_id))}
      ${safetyItem("Schedule", policy.interval_hours ? `Every ${Number(policy.interval_hours)} hours` : "Unavailable", Boolean(policy.interval_hours))}
    </div>
    <div class="runtime-archive-policy"><span>Max ${Number(policy.max_records_per_archive || 0)} records</span><span>${Number(policy.retention_count || 0)} retained local packages</span><span>${Math.round(Number(policy.max_package_bytes || 0) / 1048576)} MB limit</span><code>${escapeHtml(compactHash(policy.policy_hash))}</code></div>
    ${archives.length ? `<div class="runtime-archive-grid">${archives.slice(0, 20).map((archive) => {
      const exportable = archive.state === "verified" && ["present", "exported"].includes(archive.storage_state);
      return `<article class="runtime-archive-card ${statusTone(archive.state)}"><header><div><span>Archive ${Number(archive.archive_sequence || 0)}</span><h3>${escapeHtml(archive.file_name)}</h3></div>${statusBadge(archive.state)}</header><p>${Number(archive.record_count || 0)} records across ${Number(archive.table_count || 0)} evidence tables · ${escapeHtml(humanize(archive.storage_state))}</p><dl><div><dt>Chain</dt><dd class="mono">${escapeHtml(compactHash(archive.chain_root_hash))}</dd></div><div><dt>Manifest</dt><dd class="mono">${escapeHtml(compactHash(archive.manifest_sha256))}</dd></div><div><dt>Package</dt><dd class="mono">${escapeHtml(compactHash(archive.package_sha256))}</dd></div><div><dt>Verified</dt><dd>${escapeHtml(formatDate(archive.verified_at_utc))}</dd></div></dl><footer><span>${Number(archive.export_count || 0)} exports</span><div>${exportable ? `<button class="button secondary" type="button" data-evidence-archive-verify="${escapeHtml(archive.archive_id)}">Verify</button><button class="button secondary" type="button" data-evidence-archive-export="${escapeHtml(archive.archive_id)}" data-evidence-file-name="${escapeHtml(archive.file_name)}" data-evidence-package-hash="${escapeHtml(archive.package_sha256)}">Export</button>` : ""}</div></footer></article>`;
    }).join("")}</div>` : `<div class="runtime-empty compact"><strong>No evidence archives yet</strong><p>Create a verified archive after runtime, scheduling, or inference receipts exist.</p></div>`}
  </section>`;
}

'''
text = replace_once(text, "function renderRuntimePage() {\n", render_function + "function renderRuntimePage() {\n", "archive renderer")
text = replace_once(text, "      ${renderModelGovernance()}\n", "      ${renderModelGovernance()}\n      ${renderEvidenceArchives()}\n", "archive section")
text = replace_once(
    text,
    '    invoke("homeserver_model_governance"),\n  ]);',
    '    invoke("homeserver_model_governance"),\n    invoke("homeserver_evidence_archives"),\n  ]);',
    "archive refresh request",
)
text = replace_once(
    text,
    '  if (results[4].status === "fulfilled") runtimeState.governance = results[4].value;\n',
    '  if (results[4].status === "fulfilled") runtimeState.governance = results[4].value;\n  if (results[5].status === "fulfilled") runtimeState.archives = results[5].value;\n',
    "archive refresh assignment",
)
text = replace_once(
    text,
    '    runtimeState.governance = await invoke("homeserver_model_governance");\n',
    '    runtimeState.governance = await invoke("homeserver_model_governance");\n    runtimeState.archives = await invoke("homeserver_evidence_archives");\n',
    "archive cycle refresh",
)
actions = r'''
async function createEvidenceArchive() {
  if (runtimeState.busy) return;
  if (window.prompt("Type CREATE EVIDENCE ARCHIVE to continue:") !== "CREATE EVIDENCE ARCHIVE") return;
  runtimeState.busy = true;
  runtimeState.error = null;
  renderRuntimePage();
  try {
    const result = await invoke("homeserver_create_evidence_archive");
    runtimeState.archives = result.snapshot || result;
  } catch (error) {
    runtimeState.error = String(error);
  } finally {
    runtimeState.busy = false;
    runtimeState.lastLoadedAt = Date.now();
    renderRuntimePage();
  }
}

async function verifyEvidenceArchive(button) {
  if (runtimeState.busy) return;
  const archiveId = button.dataset.evidenceArchiveVerify || "";
  const confirmation = `VERIFY EVIDENCE ARCHIVE ${archiveId}`;
  if (window.prompt(`Type ${confirmation} to verify the encrypted package and chain:`) !== confirmation) return;
  runtimeState.busy = true;
  renderRuntimePage();
  try {
    const result = await invoke("homeserver_verify_evidence_archive", { archiveId, confirmation });
    runtimeState.archives = result.snapshot || result;
  } catch (error) {
    runtimeState.error = String(error);
  } finally {
    runtimeState.busy = false;
    runtimeState.lastLoadedAt = Date.now();
    renderRuntimePage();
  }
}

async function exportEvidenceArchive(button) {
  if (runtimeState.busy) return;
  const archiveId = button.dataset.evidenceArchiveExport || "";
  const suggestedFileName = button.dataset.evidenceFileName || "Microgifter-HomeServer-Evidence.mgha";
  const packageSha256 = button.dataset.evidencePackageHash || "";
  runtimeState.busy = true;
  renderRuntimePage();
  try {
    const result = await invoke("homeserver_export_evidence_archive", { archiveId, suggestedFileName, packageSha256 });
    if (result) runtimeState.archives = result.receipt?.snapshot || await invoke("homeserver_evidence_archives");
  } catch (error) {
    runtimeState.error = String(error);
  } finally {
    runtimeState.busy = false;
    runtimeState.lastLoadedAt = Date.now();
    renderRuntimePage();
  }
}

async function updateEvidenceArchivePolicy() {
  if (runtimeState.busy) return;
  const current = runtimeState.archives?.policy || {};
  const enabled = window.confirm("Enable automatic evidence archives? Select Cancel to disable them.");
  const intervalHours = Number(window.prompt("Archive interval in hours (1-720):", String(current.interval_hours || 24)));
  const maxRecordsPerArchive = Number(window.prompt("Maximum records per archive (100-50000):", String(current.max_records_per_archive || 5000)));
  const retentionCount = Number(window.prompt("Retained local package count (1-365):", String(current.retention_count || 30)));
  const maxPackageMb = Number(window.prompt("Maximum package size in MB (1-256):", String(Math.round(Number(current.max_package_bytes || 67108864) / 1048576))));
  const reason = window.prompt("Reason for this policy revision:", "Updated from Agent Runtime Control Center");
  if (!reason?.trim()) return;
  const policy = {
    enabled,
    interval_hours: intervalHours,
    max_records_per_archive: maxRecordsPerArchive,
    retention_count: retentionCount,
    max_package_bytes: maxPackageMb * 1048576,
    reason: reason.trim(),
  };
  runtimeState.busy = true;
  renderRuntimePage();
  try {
    const result = await invoke("homeserver_update_evidence_archive_policy", { policy });
    runtimeState.archives = result.snapshot || result;
  } catch (error) {
    runtimeState.error = String(error);
  } finally {
    runtimeState.busy = false;
    runtimeState.lastLoadedAt = Date.now();
    renderRuntimePage();
  }
}

'''
text = replace_once(text, 'document.addEventListener("click", (event) => {\n', actions + 'document.addEventListener("click", (event) => {\n', "archive actions")
handler = r'''  const updateArchivePolicy = event.target.closest("[data-evidence-policy-update]");
  if (updateArchivePolicy) {
    event.preventDefault();
    void updateEvidenceArchivePolicy();
    return;
  }
  const createArchive = event.target.closest("[data-evidence-archive-create]");
  if (createArchive) {
    event.preventDefault();
    void createEvidenceArchive();
    return;
  }
  const verifyArchive = event.target.closest("[data-evidence-archive-verify]");
  if (verifyArchive) {
    event.preventDefault();
    void verifyEvidenceArchive(verifyArchive);
    return;
  }
  const exportArchive = event.target.closest("[data-evidence-archive-export]");
  if (exportArchive) {
    event.preventDefault();
    void exportEvidenceArchive(exportArchive);
    return;
  }
'''
text = replace_once(text, '  const createPolicy = event.target.closest("[data-model-policy-create]");\n', handler + '  const createPolicy = event.target.closest("[data-model-policy-create]");\n', "archive click handlers")
write(path, text)


# Archive-specific presentation without changing the page layout system.
path = "src/agent-runtime-control-center.css"
text = read(path)
styles = r'''

.runtime-evidence-archive.safe { border-color: rgba(34, 197, 94, .24); }
.runtime-evidence-archive.unsafe { border-color: rgba(239, 68, 68, .35); }
.runtime-archive-actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.runtime-archive-actions > span { font-size: 12px; color: var(--muted); }
.runtime-archive-policy { display: flex; gap: 10px; flex-wrap: wrap; padding: 10px 12px; margin: 12px 0; border: 1px solid var(--line); border-radius: 12px; background: rgba(148, 163, 184, .06); }
.runtime-archive-policy span, .runtime-archive-policy code { font-size: 12px; color: var(--muted); }
.runtime-archive-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 12px; }
.runtime-archive-card { border: 1px solid var(--line); border-radius: 14px; padding: 14px; background: var(--panel-soft); }
.runtime-archive-card.success { border-color: rgba(34, 197, 94, .26); }
.runtime-archive-card.danger { border-color: rgba(239, 68, 68, .28); }
.runtime-archive-card header, .runtime-archive-card footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.runtime-archive-card header span, .runtime-archive-card footer > span { font-size: 11px; color: var(--muted); text-transform: uppercase; letter-spacing: .05em; }
.runtime-archive-card h3 { margin: 3px 0 0; font-size: 14px; overflow-wrap: anywhere; }
.runtime-archive-card p { margin: 10px 0; color: var(--muted); font-size: 12px; }
.runtime-archive-card dl { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin: 0 0 12px; }
.runtime-archive-card dl div { min-width: 0; }
.runtime-archive-card dt { font-size: 10px; color: var(--muted); text-transform: uppercase; }
.runtime-archive-card dd { margin: 3px 0 0; font-size: 12px; overflow-wrap: anywhere; }
.runtime-archive-card footer div { display: flex; gap: 6px; }
'''
if "runtime-evidence-archive" not in text:
    text += styles
write(path, text)


# Register permanent frontend validator.
path = "package.json"
package = json.loads(read(path))
check = package["scripts"]["check:frontend"]
if "validate-evidence-archive.py" not in check:
    check = check.replace("validate-model-inference-governance.py", "validate-model-inference-governance.py validate-evidence-archive.py")
package["scripts"]["check:frontend"] = check
write(path, json.dumps(package, indent=2) + "\n")

print("Phase 21 evidence archive integration applied.")
