use super::{client, decode_json, get_json, post_json};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rfd::AsyncFileDialog;
use serde_json::{json, Value};
use std::path::Path;
use tokio_util::io::ReaderStream;

const MAX_VAULT_DOCUMENT_BYTES: u64 = 32 * 1024 * 1024;
const FILE_NAME_HEADER: &str = "x-mg-vault-file-name";
const TAGS_HEADER: &str = "x-mg-vault-tags";

#[tauri::command]
pub(crate) async fn homeserver_vault() -> Result<Value, String> {
    get_json("/v1/vault").await
}

#[tauri::command]
pub(crate) async fn homeserver_search_vault(query: String) -> Result<Value, String> {
    post_json("/v1/vault/search", &json!({ "query": query, "limit": 20 })).await
}

#[tauri::command]
pub(crate) async fn homeserver_semantic_vault() -> Result<Value, String> {
    get_json("/v1/vault/semantic").await
}

#[tauri::command]
pub(crate) async fn homeserver_rebuild_semantic_vault(force: bool) -> Result<Value, String> {
    post_json("/v1/vault/semantic/rebuild", &json!({ "force": force })).await
}

#[tauri::command]
pub(crate) async fn homeserver_search_semantic_vault(
    query: String,
    mode: String,
) -> Result<Value, String> {
    post_json(
        "/v1/vault/semantic/search",
        &json!({ "query": query, "mode": mode, "limit": 20 }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_reindex_vault() -> Result<Value, String> {
    post_json("/v1/vault/reindex", &json!({})).await
}

#[tauri::command]
pub(crate) async fn homeserver_delete_vault_document(
    document_id: String,
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/vault/delete",
        &json!({ "document_id": document_id, "confirmation": confirmation }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_import_vault_document(
    tags: Vec<String>,
) -> Result<Option<Value>, String> {
    let Some(source) = AsyncFileDialog::new()
        .add_filter(
            "Knowledge Vault documents",
            &[
                "txt", "md", "csv", "json", "log", "pdf", "docx", "png", "jpg", "jpeg", "tif",
                "tiff",
            ],
        )
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let path = source.path();
    reject_unsafe_source(path)?;
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| error.to_string())?;
    if metadata.len() == 0 || metadata.len() > MAX_VAULT_DOCUMENT_BYTES {
        return Err("Knowledge Vault documents must be between 1 byte and 32 MB.".to_owned());
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Selected document file name is invalid.".to_owned())?;
    let input = tokio::fs::File::open(path)
        .await
        .map_err(|error| error.to_string())?;
    let encoded_name = URL_SAFE_NO_PAD.encode(file_name.as_bytes());
    let tags_json = serde_json::to_vec(&tags).map_err(|error| error.to_string())?;
    let encoded_tags = URL_SAFE_NO_PAD.encode(tags_json);
    let response = client()?
        .post(format!(
            "{}/v1/vault/import",
            microgifter_homeserver_core::api_base_url()
        ))
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .header(reqwest::header::CONTENT_LENGTH, metadata.len())
        .header(FILE_NAME_HEADER, encoded_name)
        .header(TAGS_HEADER, encoded_tags)
        .body(reqwest::Body::wrap_stream(ReaderStream::new(input)))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    decode_json(response).await.map(Some)
}

fn reject_unsafe_source(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err(
            "Knowledge Vault does not import symbolic links or reparse-point files.".to_owned(),
        );
    }
    if !metadata.is_file() {
        return Err("Knowledge Vault can only import regular files.".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximum_document_size_is_bounded() {
        assert_eq!(MAX_VAULT_DOCUMENT_BYTES, 32 * 1024 * 1024);
    }
}
