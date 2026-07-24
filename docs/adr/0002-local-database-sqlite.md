# ADR 0002: Embedded SQLite for the local operational database

- Status: Accepted for Phase 1
- Date: 2026-07-24

## Decision

Use embedded SQLite for HomeServer configuration, local service events, synchronization metadata, local indexes, automation state, audit receipts, and backup metadata.

Phase 1 enables WAL mode, foreign-key enforcement, a busy timeout, idempotent migrations, and a local sync queue.

## Rationale

- No separate database installer or customer administration.
- Reliable transactional storage for a single HomeServer node.
- Straightforward encrypted backup and migration path.
- Compatible with a one-click Windows installer.

## Boundary

SQLite is not authoritative for Microgifter payments, wallet ownership, PPPM ownership, claims, redemption, or shared commerce records.
