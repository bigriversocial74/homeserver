# Phase 4C — Local Document Extraction and OCR

## Status

Phase 4C expands the managed Knowledge Vault beyond UTF-8 text files. HomeServer extracts searchable content from PDF and DOCX documents, records page-level metadata, and can process scanned pages and images with locally installed OCR tools.

All extraction remains on the Windows HomeServer. Documents, extracted text, page images, OCR output, and search queries are not sent to Microgifter Cloud or an external OCR service.

## Supported formats

- UTF-8 text, Markdown, CSV, JSON, and log files.
- Searchable PDF documents using HomeServer's native PDF extraction layer.
- DOCX paragraphs, tables, tabs, and line breaks using bounded ZIP/XML parsing.
- PNG, JPG/JPEG, and TIFF images through local Tesseract OCR.
- Scanned PDF pages through local Poppler rendering followed by Tesseract OCR.

Macro-enabled Office files, PDF attachments, PDF JavaScript, embedded executables, symbolic links, and arbitrary archive content are not executed.

## OCR runtime

Searchable PDFs and DOCX documents work without a separate OCR installation. Scanned PDFs require both Tesseract and Poppler; standalone images require Tesseract.

HomeServer detects only fixed local executable locations, the Windows PATH, and two administrator-controlled environment overrides:

- `MG_HOMESERVER_TESSERACT_EXE`
- `MG_HOMESERVER_PDFTOPPM_EXE`

The Control Center displays these commands when the tools are unavailable. Run them from an administrator PowerShell window so the LocalSystem HomeServer service can detect the machine-wide installations:

```powershell
winget install --id tesseract-ocr.tesseract --exact --scope machine
winget install --id oschwartz10612.Poppler --exact --scope machine
```

HomeServer does not execute Winget automatically, elevate itself, download OCR binaries, or accept a caller-provided command or executable path.

## Processing lifecycle

1. The approved file is copied into managed Knowledge Vault storage.
2. A restart-safe extraction operation is recorded.
3. HomeServer validates the file type and bounded size.
4. Native PDF or DOCX extraction runs where possible.
5. Scanned pages are rendered and OCR'd only when the local tools are available.
6. Page text, extraction method, confidence, hashes, and status are stored in SQLite.
7. The keyword index is updated.
8. Existing semantic vectors are marked stale so a new cited semantic index can be built from the page-level text.

A scanned document imported before OCR tools are installed remains in managed storage with an `ocr_required` state. After installing the local tools, **Check Files** retries extraction without requiring another upload.

## Safety limits

- 32 MB maximum managed document size.
- 200 PDF pages.
- 120,000 extracted characters per page.
- 2,000,000 indexed characters per document.
- 2,048 DOCX ZIP entries.
- 16 MB maximum `word/document.xml` payload.
- 90-second PDF rendering timeout per page.
- 45-second OCR timeout per page.
- 4 MB maximum OCR stdout and 256 KB maximum diagnostic stderr.
- Temporary rendered pages are deleted after each extraction attempt.

Extraction failures expose bounded status codes and redacted diagnostics. Production logs never include document text, OCR text, embeddings, or user search queries.

## Page-aware retrieval

Extracted page records feed Phase 4D semantic indexing. PDF and image-derived matches can therefore return citations such as `policy.pdf · page 3`; formats without meaningful page boundaries retain section citations.

## Deferred work

Phase 4C does not include cloud OCR, handwriting-specific models, Office macros, password recovery for encrypted files, PDF attachment extraction, or an automatic installer for third-party OCR tools.
