use anyhow::{bail, Context, Result};
use reqwest::{header, StatusCode};
use serde_json::{json, Value};
use std::{env, time::Duration};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use zeroize::Zeroizing;

const MCP_ENDPOINT: &str = "http://127.0.0.1:47831/mcp";
const MCP_TOKEN_ENV: &str = "MG_HOMESERVER_MCP_TOKEN";
const MAX_REQUEST_BYTES: usize = 128 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[tokio::main]
async fn main() -> Result<()> {
    if env::args().any(|argument| argument == "--version") {
        println!("MicrogifterHomeServerMCP {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let token =
        Zeroizing::new(env::var(MCP_TOKEN_ENV).with_context(|| {
            format!("{MCP_TOKEN_ENV} is required for the HomeServer MCP bridge")
        })?);
    validate_token(token.as_str())?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(2 * 60))
        .build()
        .context("unable to create the local MCP HTTP client")?;

    let stdin = io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = io::stdout();
    while let Some(line) = lines
        .next_line()
        .await
        .context("unable to read MCP stdin")?
    {
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_REQUEST_BYTES {
            write_error(
                &mut stdout,
                Value::Null,
                -32600,
                "MCP request exceeded the HomeServer size limit.",
            )
            .await?;
            continue;
        }

        let request_id = serde_json::from_str::<Value>(&line)
            .ok()
            .and_then(|value| value.get("id").cloned());
        let response = client
            .post(MCP_ENDPOINT)
            .header(header::AUTHORIZATION, format!("Bearer {}", token.as_str()))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .body(line)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                eprintln!("HomeServer MCP bridge request failed: {error}");
                if let Some(id) = request_id {
                    write_error(
                        &mut stdout,
                        id,
                        -32603,
                        "The local HomeServer MCP service is unavailable.",
                    )
                    .await?;
                }
                continue;
            }
        };

        if response.status() == StatusCode::ACCEPTED {
            continue;
        }
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            if let Some(id) = request_id {
                write_error(
                    &mut stdout,
                    id,
                    -32603,
                    "The local HomeServer MCP response exceeded the size limit.",
                )
                .await?;
            }
            continue;
        }
        let bytes = response
            .bytes()
            .await
            .context("unable to read MCP response")?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            if let Some(id) = request_id {
                write_error(
                    &mut stdout,
                    id,
                    -32603,
                    "The local HomeServer MCP response exceeded the size limit.",
                )
                .await?;
            }
            continue;
        }
        if !status.is_success() {
            eprintln!("HomeServer MCP bridge received HTTP {status}");
            if let Some(id) = request_id {
                let message = if status == StatusCode::UNAUTHORIZED {
                    "The HomeServer MCP token is invalid, expired, or revoked."
                } else if status == StatusCode::TOO_MANY_REQUESTS {
                    "The HomeServer MCP client exceeded its local rate limit."
                } else {
                    "The local HomeServer MCP request failed."
                };
                write_error(&mut stdout, id, -32603, message).await?;
            }
            continue;
        }
        if bytes.is_empty() {
            continue;
        }
        stdout
            .write_all(&bytes)
            .await
            .context("unable to write MCP stdout")?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }
    Ok(())
}

fn validate_token(token: &str) -> Result<()> {
    if !token.starts_with("mghs_mcp_") || token.len() > 96 || token.chars().any(char::is_whitespace)
    {
        bail!("{MCP_TOKEN_ENV} is not a valid HomeServer MCP client token");
    }
    Ok(())
}

async fn write_error(stdout: &mut io::Stdout, id: Value, code: i64, message: &str) -> Result<()> {
    let payload = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    }))?;
    stdout.write_all(&payload).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}
