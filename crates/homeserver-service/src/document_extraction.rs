use crate::config::AppConfig;
use anyhow::{bail, ensure, Context, Result};
use lopdf::Document;
use quick_xml::{escape::unescape, events::Event, Reader};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env,
    ffi::OsString,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
use zip::ZipArchive;

const EXTRACTION_MIGRATION: &str =
    include_str!("../../../database/migrations/0008_document_extraction.sql");
const EXTRACTION_MIGRATION_KEY: &str = "0008_document_extraction";
const MAX_PAGES: usize = 200;
const MAX_PAGE_CHARS: usize = 120_000;
const MAX_EXTRACTED_CHARS: usize = 2_000_000;
const MAX_DOCX_ENTRIES: usize = 2_048;
const MAX_DOCX_XML_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOOL_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const PDF_RENDER_TIMEOUT: Duration = Duration::from_secs(90);
const OCR_PAGE_TIMEOUT: Duration = Duration::from_secs(45);
const MIN_NATIVE_PAGE_CHARS: usize = 24;
const OPERATION_HISTORY_LIMIT: i64 = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OcrRuntimeSnapshot {
    pub state: String,
    pub tesseract_available: bool,
    pub pdf_renderer_available: bool,
    pub image_ocr_available: bool,
    pub scanned_pdf_ocr_available: bool,
    pub tesseract_install_command: String,
    pub poppler_install_command: String,
    pub local_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtractionOperation {
    pub operation_id: String,
    pub document_id: Option<String>,
    pub file_name: String,
    pub operation_type: String,
    pub state: String,
    pub status_message: String,
    pub processed_pages: u32,
    pub total_pages: u32,
    pub failure_code: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub completed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocumentExtractionSummary {
    pub document_id: String,
    pub state: String,
    pub extraction_method: String,
    pub page_count: u32,
    pub native_page_count: u32,
    pub ocr_page_count: u32,
    pub ocr_required_page_count: u32,
    pub extracted_char_count: u64,
    pub confidence_permille: Option<u16>,
    pub failure_code: Option<String>,
    pub extracted_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtractionSnapshot {
    pub documents: Vec<DocumentExtractionSummary>,
    pub ready_documents: u64,
    pub partial_documents: u64,
    pub ocr_required_documents: u64,
    pub failed_documents: u64,
    pub total_pages: u64,
    pub ocr_pages: u64,
    pub runtime: OcrRuntimeSnapshot,
    pub latest_operation: Option<ExtractionOperation>,
    pub supported_extensions: Vec<String>,
    pub local_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPage {
    pub page_number: u32,
    pub extraction_method: String,
    pub text: String,
    pub confidence_permille: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionResult {
    pub state: String,
    pub extraction_method: String,
    pub pages: Vec<ExtractedPage>,
    pub native_page_count: u32,
    pub ocr_page_count: u32,
    pub ocr_required_page_count: u32,
    pub confidence_permille: Option<u16>,
    pub failure_code: Option<String>,
    pub indexed_text: String,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(EXTRACTION_MIGRATION)?;
    connection.execute(
        "UPDATE vault_extraction_operations SET state='interrupted',status_message='Interrupted by HomeServer restart',failure_code='service_restarted',completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state IN ('pending','running')",
        [],
    )?;
    connection.execute(
        "DELETE FROM vault_extraction_operations WHERE operation_id NOT IN (SELECT operation_id FROM vault_extraction_operations ORDER BY updated_at_utc DESC,operation_id DESC LIMIT ?1) AND state NOT IN ('pending','running')",
        params![OPERATION_HISTORY_LIMIT],
    )?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![EXTRACTION_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "document extraction migration is not registered exactly once"
    );
    let _: i64 = connection.query_row(
        "SELECT COUNT(*) FROM vault_document_extractions",
        [],
        |row| row.get(0),
    )?;
    let _: i64 = connection.query_row("SELECT COUNT(*) FROM vault_document_pages", [], |row| {
        row.get(0)
    })?;
    Ok(())
}

pub fn snapshot(connection: &Connection) -> Result<ExtractionSnapshot> {
    let mut statement = connection.prepare(
        "SELECT document_id,state,extraction_method,page_count,native_page_count,ocr_page_count,ocr_required_page_count,extracted_char_count,confidence_permille,failure_code,extracted_at_utc FROM vault_document_extractions ORDER BY updated_at_utc DESC,document_id DESC LIMIT 200",
    )?;
    let documents = statement
        .query_map([], extraction_summary_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let counts = connection.query_row(
        "SELECT COALESCE(SUM(CASE WHEN state='ready' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN state='partial' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN state='ocr_required' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN state='failed' THEN 1 ELSE 0 END),0),COALESCE(SUM(page_count),0),COALESCE(SUM(ocr_page_count),0) FROM vault_document_extractions",
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;
    Ok(ExtractionSnapshot {
        documents,
        ready_documents: counts.0.max(0) as u64,
        partial_documents: counts.1.max(0) as u64,
        ocr_required_documents: counts.2.max(0) as u64,
        failed_documents: counts.3.max(0) as u64,
        total_pages: counts.4.max(0) as u64,
        ocr_pages: counts.5.max(0) as u64,
        runtime: runtime_snapshot(),
        latest_operation: latest_operation(connection)?,
        supported_extensions: supported_extensions()
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        local_only: true,
    })
}

pub fn supported_extensions() -> &'static [&'static str] {
    &[
        "txt", "md", "csv", "json", "log", "pdf", "docx", "png", "jpg", "jpeg", "tif", "tiff",
    ]
}

pub fn runtime_snapshot() -> OcrRuntimeSnapshot {
    let tesseract = find_tesseract();
    let renderer = find_pdftoppm();
    let state = match (tesseract.is_some(), renderer.is_some()) {
        (true, true) => "ready",
        (true, false) => "image_only",
        (false, true) => "renderer_only",
        (false, false) => "not_installed",
    };
    OcrRuntimeSnapshot {
        state: state.to_owned(),
        tesseract_available: tesseract.is_some(),
        pdf_renderer_available: renderer.is_some(),
        image_ocr_available: tesseract.is_some(),
        scanned_pdf_ocr_available: tesseract.is_some() && renderer.is_some(),
        tesseract_install_command:
            "winget install --id tesseract-ocr.tesseract --exact --scope machine".to_owned(),
        poppler_install_command:
            "winget install --id oschwartz10612.Poppler --exact --scope machine".to_owned(),
        local_only: true,
    }
}

pub fn begin_operation(
    connection: &Connection,
    document_id: Option<&str>,
    file_name: &str,
    operation_type: &str,
) -> Result<String> {
    ensure!(
        matches!(operation_type, "import" | "reindex"),
        "unsupported extraction operation type"
    );
    let operation_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO vault_extraction_operations (operation_id,document_id,file_name,operation_type,state,status_message) VALUES (?1,?2,?3,?4,'running','Extracting local document content')",
        params![operation_id, document_id, file_name, operation_type],
    )?;
    Ok(operation_id)
}

pub fn update_operation_progress(
    connection: &Connection,
    operation_id: &str,
    processed_pages: u32,
    total_pages: u32,
) -> Result<()> {
    connection.execute(
        "UPDATE vault_extraction_operations SET processed_pages=?1,total_pages=?2,status_message='Extracting local document pages',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE operation_id=?3 AND state='running'",
        params![processed_pages as i64, total_pages as i64, operation_id],
    )?;
    Ok(())
}

pub fn finish_operation(
    connection: &Connection,
    operation_id: &str,
    state: &str,
    status_message: &str,
    failure_code: Option<&str>,
) -> Result<()> {
    ensure!(
        matches!(state, "completed" | "failed" | "interrupted"),
        "unsupported extraction completion state"
    );
    connection.execute(
        "UPDATE vault_extraction_operations SET state=?1,status_message=?2,failure_code=?3,completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE operation_id=?4",
        params![state, status_message, failure_code, operation_id],
    )?;
    Ok(())
}

pub fn extract_document<F>(
    config: &AppConfig,
    extension: &str,
    bytes: &[u8],
    managed_path: &Path,
    mut progress: F,
) -> Result<ExtractionResult>
where
    F: FnMut(u32, u32) -> Result<()>,
{
    match extension {
        "txt" | "md" | "csv" | "json" | "log" => {
            let text = extract_utf8_text(extension, bytes)?;
            progress(1, 1)?;
            Ok(result_from_pages(
                "ready",
                "text_native",
                vec![ExtractedPage {
                    page_number: 1,
                    extraction_method: "text_native".to_owned(),
                    text,
                    confidence_permille: None,
                }],
                1,
                0,
                0,
                None,
            ))
        }
        "docx" => {
            let text = extract_docx(bytes)?;
            progress(1, 1)?;
            Ok(result_from_pages(
                "ready",
                "docx_xml",
                vec![ExtractedPage {
                    page_number: 1,
                    extraction_method: "docx_xml".to_owned(),
                    text,
                    confidence_permille: None,
                }],
                1,
                0,
                0,
                None,
            ))
        }
        "pdf" => extract_pdf(config, bytes, managed_path, progress),
        "png" | "jpg" | "jpeg" | "tif" | "tiff" => extract_image(managed_path, progress),
        _ => bail!("unsupported Knowledge Vault document type"),
    }
}

pub fn store_extraction(
    connection: &Connection,
    document_id: &str,
    source_sha256: &str,
    result: &ExtractionResult,
) -> Result<()> {
    connection.execute(
        "DELETE FROM vault_document_pages WHERE document_id=?1",
        params![document_id],
    )?;
    {
        let mut statement = connection.prepare(
            "INSERT INTO vault_document_pages (document_id,page_number,extraction_method,page_text,page_text_sha256,confidence_permille) VALUES (?1,?2,?3,?4,?5,?6)",
        )?;
        for page in &result.pages {
            statement.execute(params![
                document_id,
                page.page_number as i64,
                page.extraction_method,
                page.text,
                hex::encode(Sha256::digest(page.text.as_bytes())),
                page.confidence_permille.map(i64::from),
            ])?;
        }
    }
    connection.execute(
        "INSERT INTO vault_document_extractions (document_id,state,extraction_method,source_sha256,page_count,native_page_count,ocr_page_count,ocr_required_page_count,extracted_char_count,confidence_permille,failure_code,extracted_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(document_id) DO UPDATE SET state=excluded.state,extraction_method=excluded.extraction_method,source_sha256=excluded.source_sha256,page_count=excluded.page_count,native_page_count=excluded.native_page_count,ocr_page_count=excluded.ocr_page_count,ocr_required_page_count=excluded.ocr_required_page_count,extracted_char_count=excluded.extracted_char_count,confidence_permille=excluded.confidence_permille,failure_code=excluded.failure_code,extracted_at_utc=excluded.extracted_at_utc,updated_at_utc=excluded.updated_at_utc",
        params![
            document_id,
            result.state,
            result.extraction_method,
            source_sha256,
            result.pages.len() as i64,
            result.native_page_count as i64,
            result.ocr_page_count as i64,
            result.ocr_required_page_count as i64,
            result.indexed_text.chars().count() as i64,
            result.confidence_permille.map(i64::from),
            result.failure_code,
        ],
    )?;
    Ok(())
}

pub fn page_texts(connection: &Connection, document_id: &str) -> Result<Vec<(u32, String)>> {
    let mut statement = connection.prepare(
        "SELECT page_number,page_text FROM vault_document_pages WHERE document_id=?1 AND length(page_text)>0 ORDER BY page_number",
    )?;
    let pages = statement
        .query_map(params![document_id], |row| {
            Ok((
                row.get::<_, i64>(0)?.max(1) as u32,
                row.get::<_, String>(1)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(pages)
}

fn extract_utf8_text(extension: &str, bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes).context("document must be UTF-8 text")?;
    ensure!(
        !text.contains('\0'),
        "document contains unsupported null bytes"
    );
    if extension == "json" {
        let _: serde_json::Value =
            serde_json::from_str(text).context("JSON document is invalid")?;
    }
    let text = normalize_text(text, MAX_EXTRACTED_CHARS);
    ensure!(
        !text.trim().is_empty(),
        "document contains no searchable text"
    );
    Ok(text)
}

fn extract_docx(bytes: &[u8]) -> Result<String> {
    let reader = Cursor::new(bytes);
    let mut archive = ZipArchive::new(reader).context("DOCX package is invalid")?;
    ensure!(
        archive.len() <= MAX_DOCX_ENTRIES,
        "DOCX package exceeds the entry safety limit"
    );
    let mut document = archive
        .by_name("word/document.xml")
        .context("DOCX document.xml is missing")?;
    ensure!(
        document.size() <= MAX_DOCX_XML_BYTES,
        "DOCX XML exceeds the extraction limit"
    );
    let mut xml = Vec::with_capacity(document.size().min(MAX_DOCX_XML_BYTES) as usize);
    document
        .by_ref()
        .take(MAX_DOCX_XML_BYTES + 1)
        .read_to_end(&mut xml)?;
    ensure!(
        xml.len() as u64 <= MAX_DOCX_XML_BYTES,
        "DOCX XML exceeds the extraction limit"
    );
    let text = extract_docx_xml(&xml)?;
    ensure!(!text.trim().is_empty(), "DOCX contains no searchable text");
    Ok(text)
}

fn extract_docx_xml(xml: &[u8]) -> Result<String> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut output = String::new();
    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Text(text) => {
                let decoded = text.decode()?.into_owned();
                let decoded = unescape(&decoded)?.into_owned();
                output.push_str(&decoded);
            }
            Event::Empty(element) => match element.name().as_ref() {
                b"w:tab" => output.push('\t'),
                b"w:br" | b"w:cr" => output.push('\n'),
                _ => {}
            },
            Event::End(element) => match element.name().as_ref() {
                b"w:p" | b"w:tr" => output.push('\n'),
                b"w:tc" => output.push('\t'),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        ensure!(
            output.chars().count() <= MAX_EXTRACTED_CHARS.saturating_mul(2),
            "DOCX extracted text exceeds the safety limit"
        );
        buffer.clear();
    }
    Ok(normalize_text(&output, MAX_EXTRACTED_CHARS))
}

fn extract_pdf<F>(
    config: &AppConfig,
    bytes: &[u8],
    managed_path: &Path,
    mut progress: F,
) -> Result<ExtractionResult>
where
    F: FnMut(u32, u32) -> Result<()>,
{
    let document = Document::load_mem(bytes).context("PDF document is malformed")?;
    ensure!(
        !document.is_encrypted(),
        "encrypted PDF documents are not supported"
    );
    let pages = document.get_pages();
    ensure!(!pages.is_empty(), "PDF contains no pages");
    ensure!(pages.len() <= MAX_PAGES, "PDF exceeds the 200 page limit");
    let total_pages = pages.len() as u32;
    let runtime = runtime_paths();
    let temporary = ExtractionTemp::new(config)?;
    let mut extracted_pages = Vec::with_capacity(pages.len());
    let mut native_pages = 0_u32;
    let mut ocr_pages = 0_u32;
    let mut ocr_required_pages = 0_u32;
    let mut confidences = Vec::new();
    let mut ocr_failed = false;

    for (index, page_number) in pages.keys().copied().enumerate() {
        let native = document.extract_text(&[page_number]).unwrap_or_default();
        let native = normalize_text(&native, MAX_PAGE_CHARS);
        if native
            .chars()
            .filter(|value| !value.is_whitespace())
            .count()
            >= MIN_NATIVE_PAGE_CHARS
        {
            native_pages = native_pages.saturating_add(1);
            extracted_pages.push(ExtractedPage {
                page_number,
                extraction_method: "pdf_native".to_owned(),
                text: native,
                confidence_permille: None,
            });
        } else if let (Some(renderer), Some(tesseract)) =
            (runtime.pdftoppm.as_deref(), runtime.tesseract.as_deref())
        {
            match ocr_pdf_page(
                managed_path,
                page_number,
                temporary.path(),
                renderer,
                tesseract,
            ) {
                Ok((text, confidence)) => {
                    let text = normalize_text(&text, MAX_PAGE_CHARS);
                    if text.trim().is_empty() {
                        ocr_required_pages = ocr_required_pages.saturating_add(1);
                        extracted_pages.push(ExtractedPage {
                            page_number,
                            extraction_method: "ocr_empty".to_owned(),
                            text: String::new(),
                            confidence_permille: confidence,
                        });
                    } else {
                        ocr_pages = ocr_pages.saturating_add(1);
                        if let Some(value) = confidence {
                            confidences.push(value);
                        }
                        extracted_pages.push(ExtractedPage {
                            page_number,
                            extraction_method: "pdf_ocr".to_owned(),
                            text,
                            confidence_permille: confidence,
                        });
                    }
                }
                Err(error) => {
                    ocr_failed = true;
                    ocr_required_pages = ocr_required_pages.saturating_add(1);
                    extracted_pages.push(ExtractedPage {
                        page_number,
                        extraction_method: "ocr_failed".to_owned(),
                        text: String::new(),
                        confidence_permille: None,
                    });
                    tracing::warn!(?error, page_number, "local PDF page OCR failed");
                }
            }
        } else {
            ocr_required_pages = ocr_required_pages.saturating_add(1);
            extracted_pages.push(ExtractedPage {
                page_number,
                extraction_method: "ocr_required".to_owned(),
                text: String::new(),
                confidence_permille: None,
            });
        }
        progress((index + 1) as u32, total_pages)?;
    }

    let confidence = average_confidence(&confidences);
    let searchable_pages = native_pages.saturating_add(ocr_pages);
    let (state, method, failure_code) = if searchable_pages == 0 {
        (
            "ocr_required",
            "pdf_scanned",
            Some(if ocr_failed {
                "local_ocr_page_failed"
            } else {
                "local_ocr_runtime_required"
            }),
        )
    } else if ocr_required_pages > 0 {
        (
            "partial",
            if ocr_pages > 0 {
                "pdf_hybrid_partial"
            } else {
                "pdf_native_partial"
            },
            Some(if ocr_failed {
                "local_ocr_page_failed"
            } else {
                "ocr_pages_pending"
            }),
        )
    } else if native_pages > 0 && ocr_pages > 0 {
        ("ready", "pdf_hybrid", None)
    } else if ocr_pages > 0 {
        ("ready", "pdf_ocr", None)
    } else {
        ("ready", "pdf_native", None)
    };
    Ok(result_from_pages(
        state,
        method,
        extracted_pages,
        native_pages,
        ocr_pages,
        ocr_required_pages,
        failure_code,
    )
    .with_confidence(confidence))
}

fn extract_image<F>(managed_path: &Path, mut progress: F) -> Result<ExtractionResult>
where
    F: FnMut(u32, u32) -> Result<()>,
{
    let Some(tesseract) = find_tesseract() else {
        progress(1, 1)?;
        return Ok(result_from_pages(
            "ocr_required",
            "image_scanned",
            vec![ExtractedPage {
                page_number: 1,
                extraction_method: "ocr_required".to_owned(),
                text: String::new(),
                confidence_permille: None,
            }],
            0,
            0,
            1,
            Some("local_ocr_runtime_required"),
        ));
    };
    let (text, confidence) = ocr_image(managed_path, &tesseract)?;
    progress(1, 1)?;
    let text = normalize_text(&text, MAX_PAGE_CHARS);
    if text.trim().is_empty() {
        return Ok(result_from_pages(
            "ocr_required",
            "image_ocr_empty",
            vec![ExtractedPage {
                page_number: 1,
                extraction_method: "ocr_empty".to_owned(),
                text: String::new(),
                confidence_permille: confidence,
            }],
            0,
            0,
            1,
            Some("ocr_returned_no_text"),
        )
        .with_confidence(confidence));
    }
    Ok(result_from_pages(
        "ready",
        "image_ocr",
        vec![ExtractedPage {
            page_number: 1,
            extraction_method: "image_ocr".to_owned(),
            text,
            confidence_permille: confidence,
        }],
        0,
        1,
        0,
        None,
    )
    .with_confidence(confidence))
}

fn ocr_pdf_page(
    pdf_path: &Path,
    page_number: u32,
    temporary_dir: &Path,
    renderer: &Path,
    tesseract: &Path,
) -> Result<(String, Option<u16>)> {
    let prefix = temporary_dir.join(format!("page-{page_number}"));
    let arguments = vec![
        OsString::from("-f"),
        OsString::from(page_number.to_string()),
        OsString::from("-l"),
        OsString::from(page_number.to_string()),
        OsString::from("-singlefile"),
        OsString::from("-png"),
        OsString::from("-r"),
        OsString::from("200"),
        pdf_path.as_os_str().to_owned(),
        prefix.as_os_str().to_owned(),
    ];
    let _ = run_bounded(renderer, &arguments, PDF_RENDER_TIMEOUT, 64 * 1024)
        .context("local PDF page rendering failed")?;
    let image = prefix.with_extension("png");
    ensure!(
        image.is_file(),
        "local PDF renderer did not create a page image"
    );
    ocr_image(&image, tesseract)
}

fn ocr_image(image_path: &Path, tesseract: &Path) -> Result<(String, Option<u16>)> {
    let arguments = vec![
        image_path.as_os_str().to_owned(),
        OsString::from("stdout"),
        OsString::from("-l"),
        OsString::from("eng"),
        OsString::from("--psm"),
        OsString::from("6"),
        OsString::from("tsv"),
    ];
    let output = run_bounded(
        tesseract,
        &arguments,
        OCR_PAGE_TIMEOUT,
        MAX_TOOL_OUTPUT_BYTES,
    )
    .context("local OCR failed")?;
    parse_tesseract_tsv(&output)
}

fn parse_tesseract_tsv(bytes: &[u8]) -> Result<(String, Option<u16>)> {
    let content = std::str::from_utf8(bytes).context("local OCR returned invalid UTF-8")?;
    let mut output = String::new();
    let mut last_line = None::<(String, String, String, String)>;
    let mut confidences = Vec::new();
    for line in content.lines().skip(1) {
        let fields = line.splitn(12, '\t').collect::<Vec<_>>();
        if fields.len() != 12 || fields[0] != "5" {
            continue;
        }
        let word = fields[11].trim();
        if word.is_empty() {
            continue;
        }
        let current_line = (
            fields[1].to_owned(),
            fields[2].to_owned(),
            fields[3].to_owned(),
            fields[4].to_owned(),
        );
        if last_line
            .as_ref()
            .is_some_and(|value| value != &current_line)
        {
            output.push('\n');
        } else if !output.is_empty() && !output.ends_with('\n') {
            output.push(' ');
        }
        output.push_str(word);
        last_line = Some(current_line);
        if let Ok(confidence) = fields[10].parse::<f32>() {
            if confidence.is_finite() && confidence >= 0.0 {
                confidences.push((confidence.clamp(0.0, 100.0) * 10.0).round() as u16);
            }
        }
        ensure!(
            output.chars().count() <= MAX_PAGE_CHARS.saturating_mul(2),
            "OCR page text exceeds the safety limit"
        );
    }
    Ok((output, average_confidence(&confidences)))
}

fn result_from_pages(
    state: &str,
    extraction_method: &str,
    pages: Vec<ExtractedPage>,
    native_page_count: u32,
    ocr_page_count: u32,
    ocr_required_page_count: u32,
    failure_code: Option<&str>,
) -> ExtractionResult {
    let indexed_text = join_pages(&pages);
    ExtractionResult {
        state: state.to_owned(),
        extraction_method: extraction_method.to_owned(),
        pages,
        native_page_count,
        ocr_page_count,
        ocr_required_page_count,
        confidence_permille: None,
        failure_code: failure_code.map(str::to_owned),
        indexed_text,
    }
}

impl ExtractionResult {
    fn with_confidence(mut self, confidence: Option<u16>) -> Self {
        self.confidence_permille = confidence;
        self
    }
}

fn join_pages(pages: &[ExtractedPage]) -> String {
    let mut output = String::new();
    for page in pages.iter().filter(|page| !page.text.trim().is_empty()) {
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(&format!("[Page {}]\n", page.page_number));
        output.push_str(page.text.trim());
        if output.chars().count() >= MAX_EXTRACTED_CHARS {
            break;
        }
    }
    normalize_text(&output, MAX_EXTRACTED_CHARS)
}

fn normalize_text(value: &str, limit: usize) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .take(limit)
        .collect::<String>()
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn average_confidence(values: &[u16]) -> Option<u16> {
    if values.is_empty() {
        return None;
    }
    let total = values.iter().map(|value| u64::from(*value)).sum::<u64>();
    Some((total / values.len() as u64).min(1000) as u16)
}

struct RuntimePaths {
    tesseract: Option<PathBuf>,
    pdftoppm: Option<PathBuf>,
}

fn runtime_paths() -> RuntimePaths {
    RuntimePaths {
        tesseract: find_tesseract(),
        pdftoppm: find_pdftoppm(),
    }
}

fn find_tesseract() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(r"C:\Program Files\Tesseract-OCR\tesseract.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Tesseract-OCR\tesseract.exe"),
        PathBuf::from(r"C:\Program Files\WinGet\Links\tesseract.exe"),
    ];
    find_executable_paths("MG_HOMESERVER_TESSERACT_EXE", "tesseract.exe", &candidates)
        .or_else(|| find_winget_package_executable("tesseract-ocr.tesseract", "tesseract.exe"))
}

fn find_pdftoppm() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(r"C:\Program Files\poppler\Library\bin\pdftoppm.exe"),
        PathBuf::from(r"C:\Program Files\poppler\bin\pdftoppm.exe"),
        PathBuf::from(r"C:\Program Files (x86)\poppler\Library\bin\pdftoppm.exe"),
        PathBuf::from(r"C:\Program Files\WinGet\Links\pdftoppm.exe"),
    ];
    find_executable_paths("MG_HOMESERVER_PDFTOPPM_EXE", "pdftoppm.exe", &candidates)
        .or_else(|| find_winget_package_executable("oschwartz10612.Poppler", "pdftoppm.exe"))
}

fn find_winget_package_executable(package_prefix: &str, file_name: &str) -> Option<PathBuf> {
    const MAX_DIRECTORIES: usize = 4_096;
    const MAX_DEPTH: usize = 6;
    let root = PathBuf::from(r"C:\Program Files\WinGet\Packages");
    let packages = fs::read_dir(&root).ok()?;
    let package_prefix = package_prefix.to_ascii_lowercase();
    for package in packages.flatten().take(64) {
        let package_path = package.path();
        let name = package.file_name().to_string_lossy().to_ascii_lowercase();
        if !name.starts_with(&package_prefix) || !regular_directory(&package_path) {
            continue;
        }
        let mut stack = vec![(package_path, 0_usize)];
        let mut visited = 0_usize;
        while let Some((directory, depth)) = stack.pop() {
            if visited >= MAX_DIRECTORIES || depth > MAX_DEPTH {
                break;
            }
            visited += 1;
            let entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten().take(1_024) {
                let path = entry.path();
                let metadata = match fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(_) => continue,
                };
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_file()
                    && path
                        .file_name()
                        .is_some_and(|value| value.eq_ignore_ascii_case(file_name))
                {
                    return Some(path);
                }
                if metadata.is_dir() && depth < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            }
        }
    }
    None
}

fn find_executable_paths(
    env_key: &str,
    file_name: &str,
    candidates: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(value) = env::var_os(env_key) {
        let path = PathBuf::from(value);
        if regular_executable(&path) {
            return Some(path);
        }
    }
    for path in candidates {
        if regular_executable(path) {
            return Some(path.clone());
        }
    }
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path) {
        let candidate = directory.join(file_name);
        if regular_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn regular_executable(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn regular_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn run_bounded(
    executable: &Path,
    arguments: &[OsString],
    timeout: Duration,
    maximum_output: usize,
) -> Result<Vec<u8>> {
    ensure!(
        regular_executable(executable),
        "local extraction tool is unavailable"
    );
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("OMP_THREAD_LIMIT", "1");
    let mut child = command
        .spawn()
        .context("unable to start local extraction tool")?;
    let stdout = child
        .stdout
        .take()
        .context("local extraction stdout is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("local extraction stderr is unavailable")?;
    let stdout_thread = thread::spawn(move || read_bounded(stdout, maximum_output));
    let stderr_thread = thread::spawn(move || read_bounded(stderr, 256 * 1024));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            bail!("local extraction tool exceeded its timeout");
        }
        thread::sleep(Duration::from_millis(50));
    };
    let stdout = stdout_thread
        .join()
        .map_err(|_| anyhow::anyhow!("local extraction stdout reader failed"))??;
    let _stderr = stderr_thread
        .join()
        .map_err(|_| anyhow::anyhow!("local extraction stderr reader failed"))??;
    ensure!(
        status.success(),
        "local extraction tool exited unsuccessfully"
    );
    ensure!(
        stdout.len() <= maximum_output,
        "local extraction tool output exceeds the safety limit"
    );
    Ok(stdout)
}

fn read_bounded<R: Read>(reader: R, maximum: usize) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .take(maximum.saturating_add(1) as u64)
        .read_to_end(&mut output)?;
    ensure!(
        output.len() <= maximum,
        "local extraction tool output exceeds the safety limit"
    );
    Ok(output)
}

struct ExtractionTemp {
    path: PathBuf,
}

impl ExtractionTemp {
    fn new(config: &AppConfig) -> Result<Self> {
        let root = config.data_dir.join("vault").join("extraction-temp");
        fs::create_dir_all(&root)?;
        let path = root.join(Uuid::new_v4().simple().to_string());
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ExtractionTemp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn latest_operation(connection: &Connection) -> Result<Option<ExtractionOperation>> {
    connection
        .query_row(
            "SELECT operation_id,document_id,file_name,operation_type,state,status_message,processed_pages,total_pages,failure_code,created_at_utc,updated_at_utc,completed_at_utc FROM vault_extraction_operations ORDER BY updated_at_utc DESC,operation_id DESC LIMIT 1",
            [],
            extraction_operation_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn extraction_summary_from_row(row: &Row<'_>) -> rusqlite::Result<DocumentExtractionSummary> {
    Ok(DocumentExtractionSummary {
        document_id: row.get(0)?,
        state: row.get(1)?,
        extraction_method: row.get(2)?,
        page_count: row.get::<_, i64>(3)?.max(0) as u32,
        native_page_count: row.get::<_, i64>(4)?.max(0) as u32,
        ocr_page_count: row.get::<_, i64>(5)?.max(0) as u32,
        ocr_required_page_count: row.get::<_, i64>(6)?.max(0) as u32,
        extracted_char_count: row.get::<_, i64>(7)?.max(0) as u64,
        confidence_permille: row
            .get::<_, Option<i64>>(8)?
            .map(|value| value.clamp(0, 1000) as u16),
        failure_code: row.get(9)?,
        extracted_at_utc: row.get(10)?,
    })
}

fn extraction_operation_from_row(row: &Row<'_>) -> rusqlite::Result<ExtractionOperation> {
    Ok(ExtractionOperation {
        operation_id: row.get(0)?,
        document_id: row.get(1)?,
        file_name: row.get(2)?,
        operation_type: row.get(3)?,
        state: row.get(4)?,
        status_message: row.get(5)?,
        processed_pages: row.get::<_, i64>(6)?.max(0) as u32,
        total_pages: row.get::<_, i64>(7)?.max(0) as u32,
        failure_code: row.get(8)?,
        created_at_utc: row.get(9)?,
        updated_at_utc: row.get(10)?,
        completed_at_utc: row.get(11)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docx_xml_extracts_paragraphs_tables_and_tabs() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="urn:test"><w:body><w:p><w:r><w:t>First paragraph</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Cell one</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Cell two</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;
        let text = extract_docx_xml(xml).unwrap();
        assert!(text.contains("First paragraph"));
        assert!(text.contains("Cell one"));
        assert!(text.contains("Cell two"));
    }

    #[test]
    fn tesseract_tsv_is_bounded_and_preserves_lines() {
        let tsv = b"level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t95.0\tHello\n5\t1\t1\t1\t1\t2\t0\t0\t10\t10\t85.0\tworld\n5\t1\t1\t1\t2\t1\t0\t0\t10\t10\t90.0\tAgain\n";
        let (text, confidence) = parse_tesseract_tsv(tsv).unwrap();
        assert_eq!(text, "Hello world\nAgain");
        assert_eq!(confidence, Some(900));
    }

    #[test]
    fn runtime_install_commands_are_fixed() {
        let runtime = runtime_snapshot();
        assert_eq!(
            runtime.tesseract_install_command,
            "winget install --id tesseract-ocr.tesseract --exact --scope machine"
        );
        assert_eq!(
            runtime.poppler_install_command,
            "winget install --id oschwartz10612.Poppler --exact --scope machine"
        );
    }
}
