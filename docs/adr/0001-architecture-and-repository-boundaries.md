# ADR 0001: Architecture and repository boundaries

- Status: Accepted
- Date: 2026-07-24

## Decision

`bigriversocial74/homeserver` owns the Windows installer, Tauri Control Center, native service, local API, local database, sync client, MCP gateway, Model Center, Knowledge Vault, backup, recovery, updater, diagnostics, and release workflows.

`bigriversocial74/contactform` remains authoritative for Microgifter cloud identity, commerce, campaigns, rewards, wallet, PPPM, claims, redemption, central permissions, Cloud MCP rules, device registration, and cloud synchronization endpoints.

The repositories communicate only through documented, authenticated, versioned contracts.

## Consequences

- Parallel development in each repository does not create normal file-level merge conflicts.
- Cross-repository API changes require explicit contract review.
- HomeServer cannot duplicate or override cloud-authoritative commerce rules.
