# Phase 4D — Semantic Knowledge Vault Search

## Scope

Phase 4D adds bounded local semantic indexing and cited hybrid retrieval to the existing Knowledge Vault. It uses only the default embedding model assigned in Model Center and Ollama's fixed loopback API at `127.0.0.1:11434`.

## Delivered behavior

- Deterministic bounded text chunking with overlap.
- Batched local embeddings through the approved Model Center client.
- Restart-safe semantic rebuild operations with progress and retained status.
- Per-document semantic states: indexing, ready, stale, and failed.
- Automatic stale detection when source hashes or the configured embedding model change.
- Keyword, semantic, and hybrid search modes.
- Cosine-similarity ranking and document-level result aggregation.
- Source citations using the managed file and section number; page citations are supported when Phase 4C extraction supplies page metadata.
- Control Center status, progress, model guidance, semantic metrics, and cited results.
- Keyword fallback when semantic indexing has not been configured.

## Safety boundaries

- No cloud document, query, or embedding transfer.
- Ollama requests remain fixed to `http://127.0.0.1:11434` with redirects disabled.
- Only the approved configured embedding model may be used.
- Maximum 200 documents per rebuild.
- Maximum 512,000 semantic source characters per document.
- Maximum 512 chunks per document.
- Maximum 8 embedding inputs and 12,000 characters per Ollama batch.
- Maximum 5,000 stored chunks examined per search.
- Maximum 4,096 embedding dimensions.
- Embeddings must be finite and dimensionally consistent.
- Source document text and embedding vectors are excluded from logs and service-event metadata.
- Existing browser-origin, trusted-Control-Center, loopback, backup, update, and rollback boundaries remain unchanged.

## Deferred to Phase 4C

- PDF native-text extraction.
- Scanned PDF and image OCR.
- DOCX paragraph and table extraction.
- Page-level extracted text records.
- Extraction jobs, progress, and document-format failure states.
