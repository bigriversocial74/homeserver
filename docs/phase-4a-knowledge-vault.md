
# Phase 4A — Knowledge Vault foundation

## Scope delivered by this build

- Managed local copies of approved UTF-8 text, Markdown, CSV, JSON, and log files.
- Native Tauri file selection; local API callers never provide source filesystem paths.
- 16 MB per-document limit and a two-million-character indexing ceiling.
- SHA-256 duplicate prevention.
- SQLite document metadata, tags, state, extracted text, timestamps, and future agent access-rule schema.
- Local phrase search with bounded result counts and snippets.
- Reindex checks for missing, changed, oversized, symlinked, or invalid managed files.
- Explicit `DELETE` confirmation for removing a managed copy and its index.
- Audit events for imports, reindex runs, and deletion.
- Loopback-only API and trusted Control Center header enforcement inherited from the HomeServer local API.
- Live Knowledge Vault metrics, document list, import, search, reindex, and delete controls.

## Security boundary

- Imported documents are copied into `%ProgramData%\\Microgifter\\HomeServer\\vault\\documents`.
- The selected source file is not modified or deleted.
- No source path is sent to the HomeServer service.
- No Vault content is synchronized to Microgifter Cloud.
- No LAN, browser, public HTTP, model, OCR, PDF extraction, or MCP access is enabled.
- Managed paths are constrained to the canonical Vault document directory.
- Symbolic links and non-regular files are rejected.

## Deferred Phase 4 work

- PDF and office-document parsing.
- OCR.
- Embedding generation and semantic/vector search.
- Folder watching and automatic change ingestion.
- Backup package inclusion for managed document binaries.
- Agent runtime enforcement of the access-rule schema.
- Model Center and Ollama integration.

## Acceptance targets for this foundation

- Rust formatting, service tests, strict Clippy, frontend syntax, and production frontend build.
- Idempotent migration registration.
- Duplicate, unsafe-name, unsupported-type, oversized-file, missing-file, changed-file, and explicit-delete behavior.
- Existing backup, update, cloud connector, installer, signed-update, and rollback workflows must remain green before merge.
