from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "release-v013.yml"
README = ROOT / "README.md"
NOTES = ROOT / "docs" / "releases" / "v0.1.3.md"
SIGNER = ROOT / "crates" / "homeserver-service" / "src" / "bin" / "sign-update-manifest.rs"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"release contract validation failed: {message}")


def main() -> int:
    for path in (WORKFLOW, README, NOTES, SIGNER):
        require(path.is_file(), f"required file is missing: {path.relative_to(ROOT)}")

    workflow = WORKFLOW.read_text(encoding="utf-8")
    readme = README.read_text(encoding="utf-8")
    notes = NOTES.read_text(encoding="utf-8")
    signer = SIGNER.read_text(encoding="utf-8")

    required_workflow_fragments = (
        'tags:',
        "'v*.*.*'",
        'environment: production-release',
        'contents: write',
        'WINDOWS_CODESIGN_PFX_BASE64',
        'WINDOWS_CODESIGN_PFX_PASSWORD',
        'WINDOWS_CODESIGN_EXPECTED_THUMBPRINT',
        'HOMESERVER_UPDATE_PRIVATE_KEY_BASE64',
        'HOMESERVER_UPDATE_PUBLIC_KEY_BASE64',
        'MG_HOMESERVER_RELEASE_PRIVATE_KEY_BASE64',
        'MG_HOMESERVER_RELEASE_PUBLIC_KEY_BASE64',
        'Microgifter-HomeServer-v${{ steps.release.outputs.version }}-Setup.exe',
        'Microgifter-HomeServer-Setup.exe',
        'homeserver-stable.json',
        'SHA256SUMS.txt',
        'smoke-test-installer.ps1',
        'verify-installer-release.ps1',
        'smoke-test-updater.ps1',
        'gh release create',
        '--verify-tag',
    )
    for fragment in required_workflow_fragments:
        require(fragment in workflow, f"workflow is missing required contract fragment: {fragment}")

    require(
        re.search(r"if:\s*github\.event_name != 'pull_request'", workflow) is not None,
        "production release job must not run for pull requests",
    )
    require(
        "Current release source version: `0.1.3`." in readme,
        "README must identify 0.1.3 as the current release source version",
    )
    require("# Microgifter HomeServer v0.1.3" in notes, "v0.1.3 release notes title is missing")

    required_signer_fragments = (
        'MG_HOMESERVER_RELEASE_PRIVATE_KEY_BASE64',
        'MG_HOMESERVER_RELEASE_PUBLIC_KEY_BASE64',
        'release private key does not match the configured public key',
        'URL_SAFE_NO_PAD.encode',
        'SignedUpdateManifest',
        'Microgifter-HomeServer-Setup.exe',
    )
    for fragment in required_signer_fragments:
        require(fragment in signer, f"manifest signer is missing required behavior: {fragment}")

    print("HomeServer v0.1.3 release workflow contract validation passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
