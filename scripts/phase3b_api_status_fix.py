from pathlib import Path

store = Path("crates/homeserver-service/src/update_store.rs")
text = store.read_text(encoding="utf-8")
old = '.ok_or_else(|| anyhow::anyhow!("no {} update is available", state.as_str()))'
new = '.ok_or_else(|| anyhow::anyhow!("update is not available in the {} state", state.as_str()))'
if old not in text:
    raise RuntimeError("latest update state error anchor was not found")
store.write_text(text.replace(old, new, 1), encoding="utf-8")

http = Path("crates/homeserver-service/src/http.rs")
text = http.read_text(encoding="utf-8")
regression = '''
    #[test]
    fn absent_verified_update_is_a_validation_error() {
        let error = action_error(
            "update_download_failed",
            anyhow::anyhow!("update is not available in the available state"),
        );
        assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
    }
'''
if "absent_verified_update_is_a_validation_error" not in text:
    text = text.replace("\n    #[test]\n    fn accepts_maximum_multibyte", regression + "\n    #[test]\n    fn accepts_maximum_multibyte", 1)
http.write_text(text, encoding="utf-8")
