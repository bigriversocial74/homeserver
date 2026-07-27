#!/usr/bin/env python3
"""Validate the bounded local document-extraction and OCR boundary."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ERRORS: list[str] = []


def read(path: str) -> str:
    candidate = ROOT / path
    if not candidate.is_file():
        ERRORS.append(f"required Phase 4C file is missing: {path}")
        return ""
    return candidate.read_text(encoding="utf-8")


def require(path: str, marker: str, message: str) -> None:
    if marker not in read(path):
        ERRORS.append(message)


def forbid(path: str, marker: str, message: str) -> None:
    if marker in read(path):
        ERRORS.append(message)


SERVICE = "crates/homeserver-service/src/document_extraction.rs"
VAULT = "crates/homeserver-service/src/knowledge_vault.rs"
SEMANTIC = "crates/homeserver-service/src/semantic_vault.rs"
MIGRATION = "database/migrations/0008_document_extraction.sql"
TAURI = "src-tauri/src/vault.rs"
UI = "src/main.js"

for marker in (
    'const MAX_PAGES: usize = 200',
    'const MAX_PAGE_CHARS: usize = 120_000',
    'const MAX_EXTRACTED_CHARS: usize = 2_000_000',
    'const MAX_TOOL_OUTPUT_BYTES: usize = 4 * 1024 * 1024',
    'const PDF_RENDER_TIMEOUT: Duration = Duration::from_secs(90)',
    'const OCR_PAGE_TIMEOUT: Duration = Duration::from_secs(45)',
    'Document::load_mem(bytes)',
    'ensure!(!document.is_encrypted()',
    '.by_name("word/document.xml")',
    'Command::new(executable)',
    '.stdin(Stdio::null())',
    '.env("OMP_THREAD_LIMIT", "1")',
    'regular_executable(executable)',
    'MG_HOMESERVER_TESSERACT_EXE',
    'MG_HOMESERVER_PDFTOPPM_EXE',
    'winget install --id tesseract-ocr.tesseract --exact --scope machine',
    'winget install --id oschwartz10612.Poppler --exact --scope machine',
    'local_only: true',
):
    require(SERVICE, marker, f"document extraction boundary is missing: {marker}")

for marker in (
    'document_extraction::supported_extensions()',
    'document_extraction::extract_document',
    'document_extraction::store_extraction',
    'MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024',
):
    require(VAULT, marker, f"Knowledge Vault extraction integration is missing: {marker}")

for marker in (
    'vault_document_extractions',
    'vault_document_pages',
    'vault_extraction_operations',
    'confidence_permille',
    "VALUES ('0008_document_extraction')",
):
    require(MIGRATION, marker, f"document extraction migration is missing: {marker}")

for marker in (
    'document_extraction::page_texts',
    'page_number: Some(*page_number)',
    'citation(file_name: &str, page_number: Option<u32>',
):
    require(SEMANTIC, marker, f"semantic page-citation integration is missing: {marker}")

for marker in (
    'MAX_VAULT_DOCUMENT_BYTES: u64 = 32 * 1024 * 1024',
    '"Knowledge Vault documents"',
    '"pdf"',
    '"docx"',
    '"tiff"',
):
    require(TAURI, marker, f"Control Center import boundary is missing: {marker}")

for marker in (
    'Scanned-document OCR needs local tools.',
    'data-ocr-command',
    'Native PDF text',
    'DOCX paragraphs & tables',
):
    require(UI, marker, f"Control Center extraction guidance is missing: {marker}")

for marker in (
    'cmd.exe',
    'cmd /c',
    'powershell.exe',
    'Invoke-WebRequest',
    'curl.exe',
    'https://',
    'reqwest',
    '0.0.0.0',
):
    forbid(SERVICE, marker, f"document extraction contains a disallowed execution/network boundary: {marker}")

for marker in (
    'document text',
    'page.text',
    'indexed_text',
):
    forbid(
        "docs/phase-4c-document-extraction-ocr.md",
        f"logs include {marker}",
        f"Phase 4C documentation must not authorize logging extracted content: {marker}",
    )

if ERRORS:
    for error in ERRORS:
        print(f"ERROR: {error}", file=sys.stderr)
    raise SystemExit(1)

print("Phase 4C document extraction and OCR boundaries validated.")
