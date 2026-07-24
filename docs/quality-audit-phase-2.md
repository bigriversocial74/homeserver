# Microgifter HomeServer Phase 2 Quality Audit

Date: 2026-07-24
Baseline: `main` at `573214f339833dbe849460b3386a0fef5ec4aa6d`
HomeServer branch: `feature/phase-2-cloud-pairing-sync-20260724`
Coordinated cloud PR: `bigriversocial74/contactform#1346`

## Scoring standard

A Phase 2 score of 10/10 requires owner-approved pairing, revocable scoped device identity, protected local secrets, signed and replay-resistant cloud requests, durable offline work, idempotent cloud receipts, explicit authority conflict handling, safe retries, observable connection state, complete tests, reproducible Windows builds, and verified installer behavior.

Deferred Phase 3+ backup, updater, knowledge, model, MCP, and advanced agent features do not reduce the Phase 2 score.

## Initial score: 1.8/10

| Area | Initial | Baseline finding |
| --- | ---: | --- |
| Cloud pairing and identity | 0.5 | No pairing flow, cloud device identity, scoped token, signing key, or revocation state. |
| Credential protection | 0.0 | No HomeServer cloud credentials existed and no OS-vault integration existed. |
| Synchronization durability | 3.0 | A local queue table existed, but there was no worker, claiming, retry, receipt, or conflict implementation. |
| Cloud authority enforcement | 4.0 | The blueprint documented authority boundaries, but runtime code did not enforce them. |
| Replay and request security | 0.0 | No signed requests, timestamp window, nonce tracking, or body limits. |
| Offline and retry behavior | 2.0 | Queue schema suggested future retry behavior, but it was not executable. |
| Control Center workflow | 1.0 | Microgifter connection was displayed only as a planned placeholder. |
| Testing and validation | 1.5 | Phase 1 quality gates existed, but no Phase 2 protocol, queue, pairing, or receipt tests. |
| Windows packaging | 5.0 | Phase 1 installer was proven, but no Phase 2 dependencies or runtime behavior had been built. |

Weighted baseline result: **1.8/10**

## Required fixes

1. Add owner-created, short-lived, one-time pairing codes in Microgifter cloud.
2. Add unique HomeServer device registration, scoped tokens, rotation, listing, and revocation.
3. Store only token hashes in cloud storage.
4. Store HomeServer device tokens and Ed25519 private keys only in the operating-system credential vault.
5. Sign every paired cloud request using method, path, timestamp, nonce, and body hash.
6. Reject expired timestamps, replayed nonces, invalid signatures, revoked tokens, and missing scopes.
7. Add bounded request bodies and synchronization batch sizes.
8. Add a durable local cloud-connection record and cloud receipt ledger.
9. Add idempotent local enqueue behavior and reject key reuse with different work.
10. Add atomic queue claiming, stale-claim recovery, exponential retry, and receipt validation.
11. Return claimed work to retry when cloud responses are malformed or incomplete.
12. Explicitly reject commerce, payment, claim, redemption, and ownership operations from HomeServer.
13. Add a background heartbeat and synchronization worker.
14. Add Control Center pairing, scope, status, queue, manual sync, and local disconnect controls.
15. Add deterministic PHP and Rust protocol vectors.
16. Register the additive MySQL migration in the canonical Microgifter manifest.
17. Rebuild and retest the Windows service, Tauri shell, NSIS installer, data preservation, and uninstall cycle.
18. Verify the coordinated HomeServer and Microgifter cloud PRs independently before merge.

## Current implementation status

The working branches now contain the pairing, signature, credential-vault, queue, receipt, retry, authority-boundary, cloud endpoint, migration, and Control Center implementation. The current code has not yet earned a final score because dependency canonicalization and complete CI are still running.

## Final score

Pending coordinated validation. Neither PR may merge below **10/10**.
