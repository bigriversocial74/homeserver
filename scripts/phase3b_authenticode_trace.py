from pathlib import Path

updater = Path("crates/homeserver-updater/src/main.rs")
text = updater.read_text(encoding="utf-8")
old = '''#[cfg(windows)]
fn verify_authenticode(path: &Path, expected_thumbprint: &str) -> Result<()> {
    let script = r#"$signature = Get-AuthenticodeSignature -LiteralPath $env:MG_UPDATE_FILE
if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate) { exit 20 }
[Console]::Out.Write($signature.SignerCertificate.Thumbprint)"#;
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("MG_UPDATE_FILE", path)
        .output()
        .context("unable to execute Windows Authenticode verification")?;
    ensure!(
        output.status.success(),
        "staged installer does not have a valid trusted Authenticode signature"
    );
    let actual = String::from_utf8(output.stdout)?.trim().replace(' ', "");
    ensure!(
        actual.eq_ignore_ascii_case(expected_thumbprint),
        "staged installer Authenticode signer does not match the signed manifest"
    );
    Ok(())
}
'''
new = '''#[cfg(windows)]
fn verify_authenticode(path: &Path, expected_thumbprint: &str) -> Result<()> {
    let script = r#"$exists = Test-Path -LiteralPath $env:MG_UPDATE_FILE
$signature = Get-AuthenticodeSignature -LiteralPath $env:MG_UPDATE_FILE
$status = [string]$signature.Status
$message = [string]$signature.StatusMessage
$thumbprint = if ($signature.SignerCertificate) { [string]$signature.SignerCertificate.Thumbprint } else { '' }
if ($status -ne 'Valid' -or -not $signature.SignerCertificate) {
    [Console]::Error.Write("path=$env:MG_UPDATE_FILE; exists=$exists; status=$status; message=$message; thumbprint=$thumbprint")
    exit 20
}
[Console]::Out.Write($thumbprint)"#;
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("MG_UPDATE_FILE", path)
        .output()
        .context("unable to execute Windows Authenticode verification")?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    ensure!(
        output.status.success(),
        "staged installer does not have a valid trusted Authenticode signature: {}",
        if stderr.is_empty() {
            "Windows PowerShell returned no signature details"
        } else {
            &stderr
        }
    );
    let actual = String::from_utf8(output.stdout)?.trim().replace(' ', "");
    ensure!(
        actual.eq_ignore_ascii_case(expected_thumbprint),
        "staged installer Authenticode signer does not match the signed manifest: expected {}, received {}",
        expected_thumbprint,
        actual
    );
    Ok(())
}
'''
if old not in text:
    raise SystemExit("updater Authenticode anchor not found")
updater.write_text(text.replace(old, new, 1), encoding="utf-8")

smoke = Path("scripts/smoke-test-updater.ps1")
text = smoke.read_text(encoding="utf-8")
old = '''    [pscustomobject]@{
        PlanPath = $planPath
        ArchivedInstaller = $archivedInstaller
        RollbackDirectory = $rollbackDirectory
    }
'''
new = '''    [pscustomobject]@{
        PlanPath = $planPath
        StagedInstaller = $stagedInstaller
        ArchivedInstaller = $archivedInstaller
        RollbackDirectory = $rollbackDirectory
    }
'''
if old not in text:
    raise SystemExit("plan return anchor not found")
text = text.replace(old, new, 1)
old = '''    $rollbackPlan = New-UpdatePlan -UpdateId "ci-rollback-$([guid]::NewGuid().ToString('N'))" -CurrentVersion $currentVersion -TargetVersion "9.9.9"
    $rollbackResult = Invoke-UpdatePlan -Plan $rollbackPlan
'''
new = '''    $rollbackPlan = New-UpdatePlan -UpdateId "ci-rollback-$([guid]::NewGuid().ToString('N'))" -CurrentVersion $currentVersion -TargetVersion "9.9.9"
    $stagedSignature = Get-WindowsPowerShellSignature -Path $rollbackPlan.StagedInstaller
    Write-Host "Staged Windows PowerShell Authenticode status: $($stagedSignature.status); $($stagedSignature.status_message); signer=$($stagedSignature.thumbprint)"
    if ($stagedSignature.status -ne "Valid" -or $stagedSignature.thumbprint -ne $SignerThumbprint) {
        throw "The staged CI update installer does not retain its trusted Authenticode identity"
    }
    $rollbackResult = Invoke-UpdatePlan -Plan $rollbackPlan
'''
if old not in text:
    raise SystemExit("rollback plan anchor not found")
smoke.write_text(text.replace(old, new, 1), encoding="utf-8")
