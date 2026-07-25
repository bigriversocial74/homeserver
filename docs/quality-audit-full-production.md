# HomeServer full production audit

- Repository: `bigriversocial74/homeserver`
- Baseline: `main` at `41a733926b974d0040837d43253e158abd3c120f`
- Audit branch: `audit/full-production-hardening-20260725`
- Release candidate: `0.1.3`
- Scope: all tracked source, configuration, migrations, scripts, workflows, installer hooks, UI, and documentation

## Scoring method

The score is an acceptance score for the currently implemented HomeServer phases, not a claim that future product phases are complete or that software can be mathematically perfect. A category receives 10/10 only when its explicit repository gates are implemented and verified by the Windows production-quality workflow.

| Category | Baseline | Material baseline gaps |
|---|---:|---|
| Architecture and boundaries | 9/10 | Security layer placement did not guarantee coverage of subsequently merged cloud routes. |
| Secrets and host security | 6/10 | DPAPI was not machine-scoped, unsupported platforms could preserve plaintext, and ProgramData ACLs were implicit. |
| Local API security | 7/10 | Loopback-only binding lacked a drive-by browser/state-change request boundary. |
| Cloud synchronization | 7/10 | Response, queue, retry, receipt, and history growth were not comprehensively bounded. |
| Backup and recovery | 8/10 | Archive extraction could consume excessive decompressed disk space before rejecting input. |
| Signed updates and rollback | 8/10 | Managed-path containment and atomic plan/result replacement needed stronger canonical checks. |
| Service reliability and observability | 8/10 | Daily logs had no retention and some interrupted file replacements were not recoverable. |
| Installer and Windows delivery | 8/10 | ProgramData permissions were not explicitly asserted and CI did not test the ACL contract. |
| Tests and release engineering | 8/10 | Duplicate temporary/version-specific workflows, one floating action, and no RustSec gate. |
| Documentation and operational truth | 7/10 | README status and credential-scope descriptions were stale. |
| **Baseline total** | **76/100 (7.6/10)** | Production-capable foundation with several material hardening gaps. |

## Remediation implemented in 0.1.3

### Machine secrets and filesystem permissions

- Cloud credentials and device backup keys use `CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN`.
- Unsupported platforms fail closed rather than storing disguised plaintext.
- Secret replacement preserves a recoverable previous file across interrupted renames.
- The installer removes inherited ProgramData permissions and grants full control only to LocalSystem and local administrators.
- Installed-service CI performs the credential vault self-test and validates the resulting ACL.

### Complete loopback API boundary

- The security layer wraps the fully merged main and cloud routers.
- Browser-originated requests are denied.
- State-changing requests require the trusted native Control Center marker.
- Native clients and smoke tests send the marker explicitly.
- Security response headers apply to every route.

### Bounded recovery and synchronization

- Recovery archives accept only the required regular files, reject duplicates and extra entries, validate declared sizes, and cap decompression before writing beyond limits.
- Failed or interrupted upload staging files are removed.
- Cloud response bodies, payloads, pending queues, attempts, receipts, and terminal history are bounded.
- Heartbeats deduplicate during outages rather than accumulating indefinitely.

### Update and operational safety

- Staged installer, updater, installation, data, rollback, and result paths are canonicalized and constrained to managed roots.
- Traversal and directory-overlap conditions are rejected.
- Update plan/result and credential replacement are interruption recoverable.
- Service logs older than 30 days are pruned without touching unrelated files.

### Release and maintenance controls

- The release version is synchronized across Cargo, npm, Tauri, the service, updater, installer, API, and Windows registration.
- Native release outputs are cleaned before staging.
- The installed release verifier executes the packaged service and checks the API and registry versions.
- GitHub Actions are pinned by full commit SHA and use read-only repository permissions.
- npm audit, RustSec `cargo audit`, strict Clippy, full tests, static security regression checks, installer/ACL tests, signed update tests, and rollback tests are mandatory.
- Dependabot covers Cargo, npm, and GitHub Actions.
- Temporary, duplicate, and version-specific workflows are removed.

## 10/10 acceptance gates

The final score becomes 100/100 only after all of the following are verified on the same PR head:

1. Dependency locks remain unchanged after regeneration.
2. Frontend, JSON, Python, and PowerShell syntax checks pass.
3. The static security-boundary regression script passes.
4. npm and RustSec dependency audits report no blocking vulnerability.
5. Rust formatting, full workspace tests, and strict Clippy pass.
6. Backup, recovery import/export, integrity, rollback, and cleanup tests pass.
7. Cloud contract, signature, browser rejection, queue, retry, and receipt tests pass.
8. The NSIS package installs as a LocalSystem service with restricted ProgramData ACLs.
9. The installed service binary, updater, local API, and Windows registration all report `0.1.3`.
10. Signed update, Authenticode, health confirmation, installer preservation, and automatic rollback tests pass.

## Current rescore

**Pending CI verification.** The source-level remediation targets 100/100, but the audit must not be marked 10/10 until the complete Windows workflow is green on the final PR head.
