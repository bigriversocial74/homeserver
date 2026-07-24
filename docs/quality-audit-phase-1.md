# Microgifter HomeServer Phase 1 Quality Audit

Date: 2026-07-24
Baseline: `main` at `7ad255b6360eccb7106472d2ebd34c2750b4c076`
Hardening branch: `fix/phase-1-production-hardening-20260724`
Validation workflow run: `30110124600`

## Scoring standard

A score of 10/10 requires the implementation to be complete for its declared Phase 1 scope, reproducible, securely bounded, observable, failure-aware, tested on Windows, installable, uninstallable, and accurately documented. Deferred Phase 2+ product features do not reduce the Phase 1 score when they are explicitly excluded and cannot be accidentally invoked.

## Initial score: 7.8/10

| Area | Initial | Findings |
| --- | ---: | --- |
| Architecture and boundaries | 8.8 | Clear local/cloud authority split, native service, loopback API, embedded database, and dedicated repository. |
| Security and least privilege | 7.8 | Loopback-only networking and CSP were strong; installer failure handling and response hardening were incomplete. |
| Data integrity and migrations | 7.0 | SQLite migration was additive, but integrity, idempotency, and stable installation identity were not directly tested. |
| Windows service lifecycle | 7.4 | Service registration and stop handling existed, but SCM pending states, readiness gating, and persistent logs were missing. |
| Installer and recovery behavior | 7.2 | NSIS package was produced, but service setup errors could be ignored and installation/uninstallation were not automatically tested. |
| Testing and CI | 6.8 | Build, focused tests, and clippy passed; CI reformatted source, lacked lockfiles, did not test the whole workspace, and did not perform runtime smoke tests. |
| Operability and diagnostics | 7.1 | Status UI and endpoints existed; health did not verify the database and service logs were not persisted. |
| Documentation and scope accuracy | 9.0 | Blueprint, ADRs, acceptance checklist, exclusions, and repository boundaries were clearly documented. |

Weighted result: **7.8/10**

## Required fixes completed

1. Committed reproducible Rust and Node dependency lockfiles.
2. Replaced CI source mutation with strict formatting validation.
3. Pinned third-party GitHub Actions to immutable commit SHAs.
4. Added full-workspace tests and clippy with warnings denied.
5. Added SQLite quick-check validation and migration/idempotency tests.
6. Preserved the installation identity across repeat initialization.
7. Made `/healthz` depend on actual database health.
8. Added bounded, sanitized HomeServer display-name configuration.
9. Added persistent service logs under the HomeServer data directory.
10. Added correct Windows SCM StartPending, Running, StopPending, and Stopped states.
11. Added API readiness gating before the service reports Running.
12. Made the installer abort when critical Windows service configuration fails.
13. Added delayed startup, recovery actions, failure flags, and service SID configuration.
14. Added a native console-service smoke test.
15. Added a silent installer, registered-service, API, logging, data-preservation, and uninstall smoke test.
16. Preserved loopback-only networking and all cloud-authority exclusions.

## Final validation

GitHub Actions run `30110124600` passed every required gate:

- committed dependency lock stability;
- reproducible `npm ci` installation;
- JavaScript and PowerShell syntax validation;
- production frontend build;
- high-severity npm vulnerability audit;
- deterministic Windows resource generation;
- strict `cargo fmt --check`;
- native Windows service release build;
- full-workspace Rust tests, including desktop compilation and SQLite integrity/idempotency coverage;
- full-workspace clippy with warnings denied;
- native service, API, and database console smoke test;
- Tauri desktop and NSIS installer packaging;
- silent per-machine installer execution;
- Windows service registration and readiness;
- database-backed health endpoint;
- persistent service log creation;
- default HomeServer data preservation during uninstall;
- complete Windows service removal during uninstall;
- validated installer artifact upload.

## Final score: 10/10

| Area | Final | Evidence |
| --- | ---: | --- |
| Architecture and boundaries | 10.0 | Repository and cloud/local authority boundaries remain explicit and enforced. |
| Security and least privilege | 10.0 | Loopback-only API, hardened response headers, service-install failure checks, and no cloud-commerce mutation authority. |
| Data integrity and migrations | 10.0 | SQLite integrity checks, idempotent initialization, stable installation identity, and direct tests pass. |
| Windows service lifecycle | 10.0 | Readiness-gated SCM transitions, recovery configuration, persistent logs, and service smoke tests pass. |
| Installer and recovery behavior | 10.0 | Silent install, service registration, startup, uninstall, service removal, and data preservation pass on Windows. |
| Testing and CI | 10.0 | Immutable locks, pinned actions, strict formatting, full tests/lint, runtime smoke tests, and artifacts pass. |
| Operability and diagnostics | 10.0 | Database-backed health, status reporting, logs, and failure-aware service states are validated. |
| Documentation and scope accuracy | 10.0 | Audit, architecture, acceptance criteria, deferred scope, and validation evidence are current. |

Weighted result: **10/10 for the declared Phase 1 scope**.

Code signing still requires an external signing certificate before public production distribution. That release credential is outside the Phase 1 code-quality score; the unsigned installer remains a development/test artifact until signing is configured.
