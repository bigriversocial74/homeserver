from pathlib import Path

main = Path("crates/homeserver-service/src/main.rs")
text = main.read_text(encoding="utf-8")
text = text.replace("    path::{Path, PathBuf},", "    path::PathBuf,")
text = text.replace("    use tokio::sync::oneshot;\n", "")
main.write_text(text, encoding="utf-8")

update = Path("crates/homeserver-service/src/update.rs")
text = update.read_text(encoding="utf-8")
if "use tokio::io::AsyncWriteExt;" not in text:
    insert_after = "use url::Url;\n"
    if insert_after not in text:
        raise RuntimeError("update.rs import anchor was not found")
    text = text.replace(insert_after, "use tokio::io::AsyncWriteExt;\n" + insert_after, 1)
text = text.replace(
    "        tokio::io::AsyncWriteExt::write_all(&mut output, &chunk).await?;",
    "        output.write_all(&chunk).await?;",
)
text = text.replace(
    "    tokio::io::AsyncWriteExt::sync_all(&mut output).await?;",
    "    output.sync_all().await?;",
)
if "tokio::io::AsyncWriteExt::sync_all" in text:
    raise RuntimeError("sync_all trait call was not repaired")
update.write_text(text, encoding="utf-8")
