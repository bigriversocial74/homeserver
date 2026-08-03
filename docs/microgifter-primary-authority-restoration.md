# Microgifter Primary HomeServer Authority Restoration

## Decision

Microgifter is restored as the primary customer-facing authority for HomeServer pairing, account ownership, entitlement leases, release eligibility, installer access, update authorization and update-result receipts.

HomeServer remains the private local runtime and retains control of local data, models, agents, tools, skills, knowledge, approvals, execution and audit evidence.

VP3 is optional. It may remain paired as a domain, POD or wrapper service, but HomeServer activation, normal operation and software updates do not depend on VP3.

## Forward restoration

This change does not roll the repository back to an earlier commit. It preserves every later HomeServer phase and adds a new authority declaration above the historical VP3 transition state.

The historical `homeserver_software_authority` row and VP3 identifiers are retained for audit and optional-provider compatibility. The new additive `homeserver_primary_authority` singleton declares Microgifter as the active primary authority.

No existing value is deleted or replaced from:

- HomeServer installation identity
- device identity or credential-vault references
- Microgifter provider connections
- VP3 or other wrapper connections
- entitlement leases and dataset grants
- local models, Knowledge Vault data, agents, tools or skills
- update records, rollback state or receipts
- Phase 16–23 authority, privacy, orchestration, scheduling, archive and audio state

## Runtime path

1. The customer opens Microgifter HomeServer Management.
2. Microgifter issues the existing one-time Sync Code.
3. HomeServer exchanges the code through the existing Microgifter provider adapter.
4. Microgifter returns device registration, a signed entitlement lease, merchant/site scope and capability grants.
5. HomeServer verifies and stores the lease while keeping secrets in the operating-system credential vault.
6. Update checks continue through the bundled signed updater.
7. Feature-class downloads require current Microgifter authorization; bootstrap, security and recovery behavior remains independently available under the existing policy.
8. Installation and rollback receipts are recorded locally and returned to Microgifter.

## Security boundary

Pairing does not grant broad HomeServer access. Wrapper capabilities, private knowledge selectors, agent authority, supervised actions, model routing and result egress remain governed by their existing HomeServer contracts.

Microgifter cannot read private HomeServer content unless the operator separately grants an exact capability and selector. Subscription or cloud outages do not erase local data or disable local-first HomeServer operation.

## Upgrade-system dependency

The restoration uses the existing HomeServer updater and the existing Microgifter endpoints:

- `/api/homeserver/v1/updates/authorize`
- `/api/homeserver/v1/updates/receipts`
- `/api/homeserver/latest-release.php`
- `/api/homeserver/download.php`

The coordinated Microgifter PR completes the release catalog, signed-manifest publication, rollout controls and account-facing update history around those existing endpoints. A second updater is not introduced.
