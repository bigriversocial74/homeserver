# VP3 Software Authority Boundary v1

## Authority model

VP3 is the HomeServer software control plane. It owns HomeServer licenses, registered devices, activation and entitlement leases, installer authorization, release channels, signed update manifests, update eligibility, suspension, revocation, replacement, and transfer.

Microgifter remains an independent provider connection. It owns Microgifter merchant/site assignments, dataset grants, CRM and campaign permissions, commerce and gifting synchronization, operational synchronization, and Microgifter-specific credentials and receipts.

HomeServer owns local data, models, conversations, agents, MCP, backups, tools, skills, and independently authorized wrapper connections.

## Transition behavior

The local authority record targets VP3 immediately but keeps `microgifter_legacy` as the current update gate until a verified VP3 device registration and entitlement lease complete the cutover. This prevents an incomplete VP3 deployment from disabling the only installed HomeServer.

When VP3 becomes active:

1. VP3 device and license identifiers must be present.
2. The VP3 entitlement lease must be verified and unexpired.
3. Update eligibility must be granted by VP3.
4. The signed updater, Authenticode verification, encrypted pre-update backup, exact-version health check, and rollback remain mandatory.
5. Microgifter provider state cannot grant or revoke the HomeServer software license.

## Required VP3 contract

The coordinated VP3 implementation must provide public endpoints for device registration, activation, entitlement lease refresh, heartbeat, credential rotation, suspension/revocation state, replacement, transfer, signed update manifest retrieval, and installer authorization/download.

Private keys, account passwords, API secrets, and production credentials are never stored in this repository.
