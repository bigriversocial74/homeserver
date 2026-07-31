from pathlib import Path
import re


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label} marker mismatch: {count}")
    return text.replace(old, new, 1)


privacy_path = Path("crates/homeserver-service/src/app/wrapper_privacy.rs")
text = privacy_path.read_text(encoding="utf-8")

text = replace_once(
    text,
    "#[derive(Debug, Clone)]\nstruct SelectorAuthority {",
    """#[derive(Debug, Clone, Copy)]
pub(crate) struct PrivacySubmissionRequest<'a> {
    pub connection_id: &'a str,
    pub grant_id: &'a str,
    pub grant_revision: u64,
    pub capability_key: &'a str,
    pub operation: &'a str,
    pub submitted_by_type: &'a str,
    pub submitted_by_id: &'a str,
    pub selector_id: Option<&'a str>,
    pub purpose: Option<&'a str>,
    pub output_schema: Option<&'a str>,
    pub remote_model_provider: Option<&'a str>,
}

type SelectorState = (String, i64, String, String, String, Option<String>, String);

#[derive(Debug, Clone, Copy)]
struct PrivacyIncidentEvidence<'a> {
    wrapper: Option<&'a str>,
    connection: Option<&'a str>,
    job: Option<&'a str>,
    selector: Option<&'a str>,
    severity: &'a str,
    category: &'a str,
    detail: &'a str,
    evidence: &'a str,
}

#[derive(Debug, Clone)]
struct SelectorAuthority {""",
    "SelectorAuthority insertion",
)

signature_pattern = re.compile(
    r"pub\(crate\) fn validate_job_privacy_submission\(\n"
    r"\s*connection: &Connection,\n"
    r"\s*connection_id: &str,\n"
    r"\s*grant_id: &str,\n"
    r"\s*grant_revision: u64,\n"
    r"\s*capability_key: &str,\n"
    r"\s*operation: &str,\n"
    r"\s*submitted_by_type: &str,\n"
    r"\s*submitted_by_id: &str,\n"
    r"\s*selector_id: Option<&str>,\n"
    r"\s*purpose: Option<&str>,\n"
    r"\s*output_schema: Option<&str>,\n"
    r"\s*remote_model_provider: Option<&str>,\n"
    r"\s*\) -> Result<Option<PrivacySubmissionBinding>> \{\n"
    r"\s*let knowledge = KNOWLEDGE_CAPABILITIES\.contains\(&capability_key\);"
)
signature_replacement = """pub(crate) fn validate_job_privacy_submission(
    connection: &Connection,
    request: PrivacySubmissionRequest<'_>,
) -> Result<Option<PrivacySubmissionBinding>> {
    let PrivacySubmissionRequest {
        connection_id,
        grant_id,
        grant_revision,
        capability_key,
        operation,
        submitted_by_type,
        submitted_by_id,
        selector_id,
        purpose,
        output_schema,
        remote_model_provider,
    } = request;
    let knowledge = KNOWLEDGE_CAPABILITIES.contains(&capability_key);"""
text, count = signature_pattern.subn(signature_replacement, text, count=1)
if count != 1:
    raise SystemExit(f"privacy submission signature marker mismatch: {count}")

selector_pattern = re.compile(
    r"let selector = selector_authority\(connection, &selector_id\)\?;\n"
    r"\s*ensure!\(\n"
    r"\s*selector\.connection_id == connection_id,\n"
    r"\s*\"selector belongs to a different connection\"\n"
    r"\s*\);"
)
selector_replacement = """let selector = selector_authority(connection, &selector_id)?;
    let connection_wrapper_id: String = connection.query_row(
        "SELECT wrapper_id FROM wrapper_connections WHERE connection_id=?1",
        params![connection_id],
        |row| row.get(0),
    )?;
    ensure!(
        selector.wrapper_id == connection_wrapper_id,
        "selector belongs to a different wrapper"
    );
    ensure!(
        selector.connection_id == connection_id,
        "selector belongs to a different connection"
    );"""
text, count = selector_pattern.subn(selector_replacement, text, count=1)
if count != 1:
    raise SystemExit(f"selector wrapper check marker mismatch: {count}")

complex_prefix = "let selector:Option<(String,i64,String,String,String,Option<String>,String)>=transaction.query_row"
text = replace_once(
    text,
    complex_prefix,
    "let selector: Option<SelectorState> = transaction.query_row",
    "selector state tuple",
)

incident_call_pattern = re.compile(
    r"record_incident_tx\(\n"
    r"\s*tx,\n"
    r"\s*None,\n"
    r"\s*None,\n"
    r"\s*None,\n"
    r"\s*None,\n"
    r"\s*\"high\",\n"
    r"\s*\"resource_authority_changed\",\n"
    r"\s*detail,\n"
    r"\s*&evidence,\n"
    r"\s*\)"
)
incident_call_replacement = """record_incident_tx(
        tx,
        PrivacyIncidentEvidence {
            wrapper: None,
            connection: None,
            job: None,
            selector: None,
            severity: "high",
            category: "resource_authority_changed",
            detail,
            evidence: &evidence,
        },
    )"""
text, count = incident_call_pattern.subn(incident_call_replacement, text, count=1)
if count != 1:
    raise SystemExit(f"privacy incident call marker mismatch: {count}")

incident_signature_pattern = re.compile(
    r"fn record_incident_tx\(\n"
    r"\s*tx: &Transaction<'_>,\n"
    r"\s*wrapper: Option<&str>,\n"
    r"\s*connection: Option<&str>,\n"
    r"\s*job: Option<&str>,\n"
    r"\s*selector: Option<&str>,\n"
    r"\s*severity: &str,\n"
    r"\s*category: &str,\n"
    r"\s*detail: &str,\n"
    r"\s*evidence: &str,\n"
    r"\s*\) -> Result<\(\)> \{\n"
    r"\s*tx\.execute"
)
incident_signature_replacement = """fn record_incident_tx(
    tx: &Transaction<'_>,
    incident: PrivacyIncidentEvidence<'_>,
) -> Result<()> {
    let PrivacyIncidentEvidence {
        wrapper,
        connection,
        job,
        selector,
        severity,
        category,
        detail,
        evidence,
    } = incident;
    tx.execute"""
text, count = incident_signature_pattern.subn(incident_signature_replacement, text, count=1)
if count != 1:
    raise SystemExit(f"privacy incident signature marker mismatch: {count}")

privacy_path.write_text(text, encoding="utf-8")

submit_path = Path("crates/homeserver-service/src/app/wrapper_jobs_submit.rs")
submit = submit_path.read_text(encoding="utf-8")
submit_pattern = re.compile(
    r"let privacy_binding = wrapper_privacy::validate_job_privacy_submission\(\n"
    r"\s*&connection,\n"
    r"\s*&connection_id,\n"
    r"\s*&grant_id,\n"
    r"\s*authorization\.grant_revision,\n"
    r"\s*&capability_key,\n"
    r"\s*&operation,\n"
    r"\s*&submitted_by_type,\n"
    r"\s*&submitted_by_id,\n"
    r"\s*request\.private_selector_id\.as_deref\(\),\n"
    r"\s*request\.purpose\.as_deref\(\),\n"
    r"\s*request\.output_schema\.as_deref\(\),\n"
    r"\s*request\.remote_model_provider\.as_deref\(\),\n"
    r"\s*\)\?;"
)
submit_replacement = """let privacy_binding = wrapper_privacy::validate_job_privacy_submission(
        &connection,
        wrapper_privacy::PrivacySubmissionRequest {
            connection_id: &connection_id,
            grant_id: &grant_id,
            grant_revision: authorization.grant_revision,
            capability_key: &capability_key,
            operation: &operation,
            submitted_by_type: &submitted_by_type,
            submitted_by_id: &submitted_by_id,
            selector_id: request.private_selector_id.as_deref(),
            purpose: request.purpose.as_deref(),
            output_schema: request.output_schema.as_deref(),
            remote_model_provider: request.remote_model_provider.as_deref(),
        },
    )?;"""
submit, count = submit_pattern.subn(submit_replacement, submit, count=1)
if count != 1:
    raise SystemExit(f"privacy submission call marker mismatch: {count}")
submit_path.write_text(submit, encoding="utf-8")
