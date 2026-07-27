use serde::Serialize;
use std::process::Command;

const OLLAMA_WINDOWS_PAGE: &str = "https://ollama.com/download/windows";
const OLLAMA_SETUP_URL: &str = "https://ollama.com/download/OllamaSetup.exe";
const OLLAMA_INSTALL_COMMAND: &str = "irm https://ollama.com/install.ps1 | iex";

#[derive(Debug, Serialize)]
pub(crate) struct OllamaSetupLaunchResult {
    launched: bool,
    target: &'static str,
    message: String,
}

#[cfg(windows)]
fn launch_url(url: &'static str, target: &'static str) -> Result<OllamaSetupLaunchResult, String> {
    Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn()
        .map_err(|error| format!("Unable to open the official Ollama {target}: {error}"))?;
    Ok(OllamaSetupLaunchResult {
        launched: true,
        target,
        message: format!("Opened the official Ollama {target} in your default browser."),
    })
}

#[cfg(not(windows))]
fn launch_url(_url: &'static str, _target: &'static str) -> Result<OllamaSetupLaunchResult, String> {
    Err("The Ollama Windows setup assistant is only available on Windows.".to_owned())
}

#[tauri::command]
pub(crate) fn homeserver_open_ollama_official(
    target: String,
) -> Result<OllamaSetupLaunchResult, String> {
    match target.as_str() {
        "installer" => launch_url(OLLAMA_SETUP_URL, "installer"),
        "documentation" => launch_url(OLLAMA_WINDOWS_PAGE, "Windows documentation"),
        _ => Err("Unsupported Ollama setup target.".to_owned()),
    }
}

#[tauri::command]
pub(crate) fn homeserver_open_ollama_terminal() -> Result<OllamaSetupLaunchResult, String> {
    #[cfg(windows)]
    {
        let message = format!(
            "Write-Host 'Microgifter HomeServer Ollama Setup' -ForegroundColor Cyan; Write-Host ''; Write-Host 'The official install command has been copied by HomeServer.'; Write-Host 'Paste it here and press Enter when you are ready:'; Write-Host ''; Write-Host '{}' -ForegroundColor DarkGray",
            OLLAMA_INSTALL_COMMAND.replace('`', "``").replace('\'', "''")
        );
        Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", &message])
            .spawn()
            .map_err(|error| format!("Unable to open PowerShell: {error}"))?;
        Ok(OllamaSetupLaunchResult {
            launched: true,
            target: "powershell",
            message: "Opened PowerShell. Paste the copied official Ollama install command when ready.".to_owned(),
        })
    }

    #[cfg(not(windows))]
    {
        Err("The Ollama PowerShell setup assistant is only available on Windows.".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_targets_are_fixed_https_urls() {
        assert_eq!(OLLAMA_SETUP_URL, "https://ollama.com/download/OllamaSetup.exe");
        assert_eq!(OLLAMA_WINDOWS_PAGE, "https://ollama.com/download/windows");
        assert_eq!(OLLAMA_INSTALL_COMMAND, "irm https://ollama.com/install.ps1 | iex");
    }
}
