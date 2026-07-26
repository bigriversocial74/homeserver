from __future__ import annotations

import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "internal-test-v013.yml"
GUIDE = ROOT / "docs" / "internal-test-v0.1.3.md"
SIGNER = ROOT / "crates" / "homeserver-service" / "src" / "bin" / "sign-update-manifest.rs"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"internal test contract validation failed: {message}")


def main() -> int:
    for path in (WORKFLOW, GUIDE, SIGNER):
        require(path.is_file(), f"required file is missing: {path.relative_to(ROOT)}")

    workflow = WORKFLOW.read_text(encoding="utf-8")
    guide = GUIDE.read_text(encoding="utf-8")
    signer = SIGNER.read_text(encoding="utf-8")

    required_workflow_fragments = (
        "RUN ANYWAY",
        "New-SelfSignedCertificate",
        "Microgifter HomeServer Internal Test",
        "Cert:\\LocalMachine\\Root",
        "Cert:\\LocalMachine\\TrustedPublisher",
        "--generate-test-key-pair",
        "MG_HOMESERVER_RELEASE_PRIVATE_KEY_BASE64",
        "MG_HOMESERVER_RELEASE_PUBLIC_KEY_BASE64",
        "Microgifter-HomeServer-v${{ steps.release.outputs.version }}-Internal-Test-Setup.exe",
        "homeserver-internal-test.json",
        "INTERNAL-TEST-WARNING.txt",
        "SHA256SUMS.txt",
        "smoke-test-installer.ps1",
        "verify-installer-release.ps1",
        "smoke-test-updater.ps1",
        "actions/upload-artifact@",
        "permissions:\n  contents: read",
    )
    for fragment in required_workflow_fragments:
        require(fragment in workflow, f"workflow is missing required fragment: {fragment}")

    forbidden_workflow_fragments = (
        "gh release create",
        "contents: write",
        "environment: production-release",
        "WINDOWS_CODESIGN_PFX_BASE64",
        "HOMESERVER_UPDATE_PRIVATE_KEY_BASE64",
    )
    for fragment in forbidden_workflow_fragments:
        require(fragment not in workflow, f"internal workflow contains forbidden production behavior: {fragment}")

    require("More info → Run anyway" in guide, "tester guide must explain the Windows Run anyway step")
    require("small, known test group" in guide, "tester guide must limit distribution")
    require("--generate-test-key-pair" in signer, "manifest signer must generate ephemeral test keys")
    require("SigningKey::generate" in signer, "test keys must be generated cryptographically")

    print("HomeServer v0.1.3 internal test workflow contract validation passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
