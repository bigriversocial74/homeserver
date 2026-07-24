# Microgifter HomeServer Phase 1 Quality Audit

Date: 2026-07-24
Baseline: `main` at `7ad255b6360eccb7106472d2ebd34c2750b4c076`
Hardening branch: `fix/phase-1-production-hardening-20260724`

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

## Required fixes

1. Commit reproducible Rust and Node dependency lockfiles.
2. Replace CI source mutation with strict formatting validation.
3. Pin third-party GitHub Actions to immutable commit SHAs.
4. Run full-workspace tests and clippy with warnings denied.
5. Add SQLite quick-check validation and migration/idempotency tests.
6. Preserve the installation identity across repeat initialization.
7. Make `/healthz` depend on actual database health.
8. Add bounded, sanitized HomeServer display-name configuration.
9. Add persistent service logs under the HomeServer data directory.
10. Report correct Windows SCM pending and running states.
11. Wait for API readiness before reporting the service as running.
12. Abort installation if critical Windows service configuration fails.
13. Configure delayed startup, recovery actions, failure flags, and service SID behavior.
14. Add a native console-service smoke test.
15. Add a silent installer, registered-service, API, logging, data-preservation, and uninstall smoke test.
16. Retain loopback-only networking and all cloud-authority exclusions.

## Current remediation status

All listed code and validation changes are implemented on the hardening branch. Final scoring remains pending until the Windows production-quality workflow proves:

- dependency lock generation and reproducible install;
- strict frontend, PowerShell, and Rust syntax/format quality;
- full-workspace Rust tests and clippy;
- native service startup and database-backed health;
- Tauri and NSIS packaging;
- silent per-machine install;
- Windows service registration and readiness;
- persistent service logging;
- default data preservation during uninstall;
- complete Windows service removal during uninstall.

## Final score

Pending live Windows validation. The branch must not merge below **10/10**.
