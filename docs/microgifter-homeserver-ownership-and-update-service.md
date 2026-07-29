# Microgifter HomeServer Ownership, Connection Authority, and Update Service Architecture

**Status:** Approved architecture and implementation baseline  
**Audience:** Microgifter, HomeServer, POD, deployment, security, and support engineering  
**Current HomeServer baseline:** `d2cb42a657730b9a182fc6bca80a963599146543`  
**Primary decision:** The Microgifter account owns the Microgifter HomeServer connection.  
**Local ownership boundary:** The HomeServer operator retains control of the local installation, local data, local models, local documents, local backups, and unrelated wrapper connections.

---

## 1. Purpose

This document is the canonical architecture specification for:

1. HomeServer connection ownership.
2. Device identity and account authority.
3. Merchant and site assignment.
4. Multi-wrapper isolation.
5. Pairing, synchronization, entitlement, and update separation.
6. Signed installer and updater trust.
7. Subscription, revocation, replacement, and recovery behavior.
8. Provider-side responsibilities that remain outside the local HomeServer runtime.

It replaces speculative architecture language with three explicit classifications:

- **Implemented:** Present in the HomeServer product baseline.
- **Required provider contract:** Must be enforced by Microgifter or another paired provider.
- **Planned:** Product or operational work that is not yet represented as a completed production capability.

---

## 2. Implementation anchors

The architecture is grounded in the following merged HomeServer work:

| Area | Implementation anchor | State |
|---|---|---|
| Multi-connection cloud registry | PR #32 | Implemented |
| Supervised Agent Workspace and approvals | PR #33 | Implemented |
| Operational data import and provenance | PR #34 | Implemented |
| Review intelligence and authorized campaign actions | PR #35 | Implemented |
| Microgifter entitlement and update client foundation | PR #37, merge `3067904feb5f2a5566673087b694e94d7fecce0e` | Implemented |
| Agent Chat and connection interface | PR #38 | Implemented |
| Native POD provider voice adapter | PR #39 | Implemented |
| Responsive observer-free Control Center lifecycle | PR #41 | Implemented |
| Windows tray and background Control Center | PR #42, merge `d2cb42a657730b9a182fc6bca80a963599146543` | Implemented |

The implementation anchors do not collapse provider authority into HomeServer. They establish the local client, local trust boundaries, connection records, signed entitlements, update authorization checks, and independent wrapper operation.

---

## 3. Permanent architecture decisions

### 3.1 Microgifter connection owner

> **The Microgifter account owns the Microgifter HomeServer connection.**

The connection is account-bound, not person-bound. The user who enters a Sync Code acts for the account but does not personally become the owner of the pairing.

Employee departure, administrator replacement, password changes, or role changes must not silently destroy the device registration.

### 3.2 Local HomeServer ownership

Pairing does not transfer ownership of the computer, local runtime, local models, or local data to Microgifter.

The HomeServer operator retains control of:

- Knowledge Vault documents
- Local model files
- Local prompts and responses
- Local conversations
- Local agent plans and reports
- Local configuration
- Local backups and recovery packages
- Local audit history
- Data belonging to other paired wrappers

Microgifter receives only explicitly authorized data and actions through a connection-scoped contract.

### 3.3 Provider isolation

Microgifter is one provider or wrapper connection. It is not the owner of HomeServer as a whole.

Each provider connection has separate:

- External account ownership
- Provider identity
- Device credentials
- Entitlements
- Capability grants
- Merchant and site assignments
- Synchronization state
- Imported evidence
- Action authority
- Receipts
- Revocation state

Revoking Microgifter must not disconnect a POD, accounting provider, communications provider, or future authorized wrapper.

### 3.4 Update trust is independent

Pairing and subscription entitlement may determine commercial eligibility, rollout, or feature access. They do not replace cryptographic update verification.

The update trust chain remains independent and requires:

- Fixed trusted manifest location
- Pinned release verification key
- Signed manifest
- Exact version and installer metadata
- SHA-256 verification
- Windows Authenticode verification
- Approved signer verification
- Pre-update backup
- Exact post-install health and version verification
- Automatic rollback on failure

### 3.5 One installer

The updater is a separate native helper component, but it is bundled inside the normal HomeServer installer.

Customers do not install a second updater product.

The standard installer includes:

- HomeServer LocalSystem service
- HomeServer Control Center
- HomeServer updater helper
- HomeServer MCP bridge
- Required local resources

---

## 4. Ownership domains

| Domain | Authoritative owner or authority |
|---|---|
| Microgifter pairing and connection | Owning Microgifter account |
| Microgifter subscription entitlement | Microgifter account and package system |
| Merchant operational records | Microgifter merchant account |
| Merchant and site assignments | Owning Microgifter account, bounded by authorization |
| Local HomeServer installation | HomeServer operator |
| Local Knowledge Vault and model data | HomeServer operator |
| POD connection | Owning POD account or authorized POD operator |
| Other wrapper connections | Their respective external accounts |
| HomeServer software update trust | Pinned HomeServer release key and signed release process |
| Windows installer identity | Approved Authenticode publisher |

No domain grants unlimited authority over another.

---

## 5. Connection hierarchy

The Microgifter relationship is:

```text
Microgifter Account
        ↓
Microgifter HomeServer Connection
        ↓
Registered HomeServer Device Identity
        ↓
Assigned Merchant Organizations
        ↓
Assigned Locations and Sites
        ↓
Approved Datasets, Capabilities, Tools, and Agent Actions
```

A Microgifter account may own multiple registered HomeServer devices when its package permits it.

A HomeServer may support multiple authorized merchants and locations while retaining strict tenant separation.

A HomeServer may also support multiple independent provider connections through the provider-neutral connection registry.

---

## 6. Implemented HomeServer responsibilities

The current HomeServer baseline implements or preserves the following local responsibilities.

### 6.1 Local-first operation

- HomeServer operates without requiring an active Microgifter connection.
- Local data, models, agents, backups, and Knowledge Vault remain locally controlled.
- A failed or revoked provider does not disable unrelated providers.
- The LocalSystem service remains independent of the Control Center UI.

### 6.2 Provider-neutral connection registry

- Multiple cloud or wrapper connections may coexist.
- Each connection has its own lifecycle and credentials.
- Provider adapters are explicit and allowlisted.
- Arbitrary provider endpoints are not accepted as trusted adapters.

### 6.3 Microgifter Phase 6A client

The merged Phase 6A client includes:

- Sync Code pairing flow
- Permanent device identity
- Explicit connection lifecycle state
- Signed entitlement leases
- Capability negotiation
- Merchant and site assignments
- Privacy-safe heartbeat and status
- Credential rotation
- Device replacement state
- Entitlement-aware update authorization
- Update channel and maintenance-window preferences
- Local receipts
- Provider contract fixtures and validation

### 6.4 Signed updater and recovery

HomeServer retains:

- Signed manifest verification
- Installer hash and size verification
- Authenticode verification
- Trusted signer checks
- Pre-update backup
- Silent installer application through the updater helper
- Service restart
- Exact target-version health verification
- Automatic rollback
- Durable update result recording

### 6.5 Operational data and supervised actions

HomeServer supports explicit connection-scoped grants for structured operational evidence.

The local system may analyze permitted data and prepare supervised actions, but connected providers remain authoritative for live cloud records and provider-side policy enforcement.

### 6.6 Windows desktop shell

The production Control Center now includes:

- Native Windows tray icon
- Close-to-tray behavior
- Optional Start with Windows setting
- Hidden startup mode
- Tray navigation to Dashboard, Agent Chat, Status, and Updates
- Explicit Quit Control Center action
- Console-free production launch

Quitting the Control Center does not stop the HomeServer LocalSystem service.

---

## 7. Required Microgifter provider contract

The following responsibilities belong to Microgifter or its coordinated provider service.

### 7.1 Account ownership

Microgifter must maintain a durable account or organization identifier as the root owner of the Microgifter connection.

Recommended records include:

- `owner_account_id`
- `created_by_user_id`
- `provider_connection_id`
- `device_id`
- `device_display_name`
- `merchant_scope`
- `site_scope`
- `capability_grants`
- `subscription_state`
- `update_eligibility`
- `revocation_state`
- `last_heartbeat`
- `last_credential_rotation`

`created_by_user_id` is an audit reference and must never replace `owner_account_id` as the ownership root.

### 7.2 Sync Code issuance

A Sync Code must be:

- One-time use
- Short-lived
- Random and unguessable
- Bound to the owning account
- Bound to an intended pairing request
- Bound to a provider and connection type
- Rate-limited
- Audited
- Invalidated immediately after successful exchange

The Sync Code is not a permanent credential.

### 7.3 Entitlement issuance

Microgifter must issue signed, time-bounded entitlement information describing:

- Account identity
- Device identity
- Subscription state
- Granted and denied capabilities
- Merchant scope
- Site scope
- Device allowance
- Update eligibility
- Allowed update channels
- Minimum supported HomeServer version when applicable

### 7.4 Dataset and action authorization

Microgifter remains authoritative for:

- Which merchant records may be exported
- Which datasets are granted
- Which sites are in scope
- Which campaign or commerce actions are permitted
- Whether a provider-side approval is still required
- Final provider-side policy enforcement

A local HomeServer approval does not bypass Microgifter authorization.

### 7.5 Revocation and replacement

Microgifter must support connection-specific:

- Credential revocation
- Capability revocation
- Dataset grant revocation
- Device replacement
- Account ownership continuity
- Audit retention

Revocation must not remotely uninstall HomeServer or erase local data.

---

## 8. Pairing, synchronization, entitlement, and updates remain separate

### 8.1 Initial installer delivery

**Required provider contract:** A paid or otherwise entitled account may receive the initial installer through an authenticated product or subscription flow.

### 8.2 Pairing

**Implemented locally and required provider-side:** Pairing exchanges a one-time Sync Code for durable device identity and protected connection credentials.

### 8.3 Synchronization

**Implemented locally and required provider-side:** Synchronization uses device credentials, connection scope, dataset grants, cursors, signed payloads, and receipts.

### 8.4 Entitlement

**Implemented locally and required provider-side:** Signed entitlement leases control commercial capabilities without becoming the update-signing trust root.

### 8.5 Software updates

**Implemented locally; publication service remains operational work:** Updates require signed release metadata and verified installers. Provider entitlement may influence feature-update eligibility but cannot authorize an unsigned or untrusted installer.

---

## 9. Update service architecture

The production update distribution boundary is:

```text
https://updates.microgifter.com/
```

The stable HomeServer manifest contract is:

```text
https://updates.microgifter.com/homeserver/stable/manifest.json
```

The update distribution service must remain operationally separate from routine Microgifter website deployments.

Recommended production structure:

```text
microgifter.com
    Account, subscription, download, and HomeServer management UI

Microgifter provider API
    Pairing, device, entitlement, assignments, synchronization, and receipts

updates.microgifter.com
    Signed manifests, immutable installers, checksums, and release notes
```

The update host should support:

- Stable, Beta, and Preview channels
- Immutable versioned installers
- Signed manifests
- SHA-256 metadata
- Release notes
- Protected publication permissions
- Safe cache behavior
- Independent monitoring and recovery
- Staged rollout and withdrawal

A routine PHP or application deployment must not alter the pinned update key, update hostname, or published immutable installer artifacts.

---

## 10. Update eligibility policy

Recommended update classes:

| Update class | Eligibility policy |
|---|---|
| Critical security or recovery | Legitimate installations, including temporarily offline or suspended devices where possible |
| Maintenance | Paired devices and payment-grace accounts |
| Feature | Active entitled accounts |
| Beta or Preview | Explicitly opted-in entitled accounts |

A canceled subscription must not erase the local installation or local data.

A suspended subscription may pause paid synchronization and feature eligibility after a grace period while preserving local operation and recovery capability.

---

## 11. Multi-device and multi-merchant behavior

### 11.1 Multiple devices

Each registered HomeServer device must have separate:

- Device identity
- Credentials
- Assignments
- Dataset grants
- Capabilities
- Synchronization state
- Update state
- Receipts
- Revocation state

Revoking one device must not affect another device owned by the same account.

### 11.2 Multiple merchants

When one HomeServer serves multiple merchants, tenant isolation applies to:

- Imported records
- Search results
- Agent context
- Plans and approvals
- Action requests
- Receipts
- Audit logs

Merchant A data must never become visible to Merchant B merely because both use the same physical HomeServer.

---

## 12. Subscription, suspension, and cancellation

Subscription state changes entitlement, not historical ownership.

During suspension or cancellation:

- The account remains the recorded connection owner.
- Pairing and audit history remain associated with the account.
- Local HomeServer operation continues.
- Local data remains intact.
- New downloads or pairings may be blocked.
- Paid synchronization may enter grace and then pause.
- Paid datasets or agent actions may pause.
- Security and recovery updates remain available where policy permits.

Reactivation should restore authorized services without requiring a new pairing unless credentials were revoked or compromised.

---

## 13. Device replacement

Device replacement transfers an account’s device allowance to a new installation without silently copying local private data.

Required process:

1. The account starts device replacement.
2. The existing device enters a replacement state.
3. A replacement Sync Code is issued.
4. The new HomeServer establishes a new device identity.
5. Merchant, site, dataset, and capability assignments are reviewed.
6. New credentials become active.
7. Old credentials are revoked.
8. Old records remain available for audit.
9. Local data moves only through an explicit encrypted backup and recovery process.

---

## 14. Revocation

Revocation disables one provider connection for one device.

Revocation must:

- Invalidate that provider’s device credentials
- Reject future synchronization for that connection
- Reject new provider actions
- Revoke or suspend provider grants
- Preserve provider and local audit history
- Preserve local HomeServer data
- Preserve unrelated wrapper connections
- Preserve recovery access where policy permits

Revocation must not remotely uninstall HomeServer, delete local backups, erase Knowledge Vault data, or stop unrelated provider connections.

---

## 15. Required connection states

The product and provider contract should distinguish:

| State | Meaning |
|---|---|
| Unpaired | No durable provider connection exists |
| Pairing pending | Pairing started but not completed |
| Active | Paired and authorized |
| Offline | Connection temporarily unreachable |
| Grace | Temporary entitlement or billing tolerance |
| Suspended | Paid capabilities paused |
| Replacing | Device replacement in progress |
| Revoked | Credentials permanently invalidated |
| Error | Connection requires repair |

Offline is not revoked. Suspended is not disconnected. Payment failure is not ownership transfer.

---

## 16. Deployment protection

A normal Microgifter deployment must not:

- Regenerate device identities
- Delete pairing records
- Rotate credentials without a controlled handshake
- Reset synchronization cursors
- Reassign merchant or site scopes
- Reassign update channels
- Change the pinned release key
- Change the update manifest hostname
- Revoke devices merely because they were temporarily unreachable

Deployment verification should prove:

```text
Existing device can authenticate
Existing pairing remains valid
Heartbeat contract remains available
Synchronization contract remains compatible
Entitlement verification remains valid
Signed update manifest remains reachable
Published installer remains reachable
No device grants were reset
No update channel changed
```

---

## 17. Audit and receipts

Durable receipts are required for security-relevant lifecycle events, including:

- Installer download authorization
- Pairing request creation
- Sync Code exchange
- Device registration
- Credential rotation
- Merchant assignment
- Site assignment
- Dataset grant change
- Capability change
- Entitlement change
- Heartbeat or lifecycle transition
- Update authorization
- Update download
- Update installation
- Update rollback
- Device replacement
- Device revocation
- Account ownership transfer

Receipts should record the applicable account, acting user, provider, connection, device, merchant/site scope, event type, request ID, previous state, new state, timestamp, result, and failure category.

---

## 18. Implementation status

### Implemented in HomeServer

- Local-first operation
- Provider-neutral multi-connection registry
- Connection-scoped credentials and state
- Microgifter Sync Code client flow
- Permanent device identity
- Signed entitlement lease verification
- Capability and assignment storage
- Heartbeat and lifecycle processing
- Credential rotation support
- Device replacement state
- Entitlement-aware update authorization
- Maintenance-window preferences
- Local receipts
- Signed updater, installer verification, backup, health check, and rollback
- Operational data grants and provenance
- Supervised local agent approvals
- Native POD provider adapter
- Windows tray and background Control Center

### Required provider-side completion or continued operation

- Durable Microgifter account ownership records
- Authenticated installer-download entitlement
- Sync Code issuance and exchange service
- Signed entitlement issuance
- Merchant and site assignment authority
- Signed synchronization exports
- Provider-side action enforcement
- Revocation and replacement administration
- Provider receipts and audit presentation

### Planned operational and product work

- Production-grade account-facing HomeServer management portal
- Production publication and rollout controls for `updates.microgifter.com`
- Formal release-key rotation and emergency recovery runbook
- Customer-facing first-run onboarding and connection-health workflow
- Consolidated support diagnostics and redacted support package

Planned work must preserve every boundary in this specification.

---

## 19. Acceptance criteria

The architecture is considered enforced when all applicable items are proven:

1. The Microgifter account remains the durable owner of the Microgifter connection.
2. Removing the user who performed pairing does not silently disconnect the device.
3. Pairing produces independent device credentials.
4. HomeServer remains locally operational without Microgifter.
5. Revoking Microgifter does not affect unrelated wrappers.
6. Merchant and site data remain tenant-isolated.
7. Signed entitlements cannot replace update-signature verification.
8. The updater helper is installed with HomeServer and is not a separate customer product.
9. Installer hash, release signature, Authenticode identity, and target version are verified.
10. Failed updates trigger automatic rollback.
11. Subscription suspension does not erase local data.
12. Device replacement preserves audit history and does not automatically copy private local data.
13. Routine website deployments do not reset pairing, device identity, grants, or update-channel state.
14. Security-relevant lifecycle changes create durable receipts.
15. The Control Center may close or exit while the LocalSystem service continues independently.

---

## 20. Permanent architecture rule

> **The Microgifter account owns and administers the Microgifter HomeServer connection. The HomeServer operator retains control of the local runtime and local data. Merchants and sites are assigned scopes under the account-owned connection. Individual administrators act for the account but do not personally own the pairing. Other paired wrappers remain independently owned and controlled. Pairing, synchronization, entitlement, and signed software updates remain separate systems. The updater is bundled with HomeServer, while signed update distribution operates independently through `updates.microgifter.com`.**

This rule protects:

- Pairing continuity through employee turnover
- Local privacy and ownership
- Multiple merchant isolation
- Multiple wrapper coexistence
- Provider-specific revocation
- Safe device replacement
- Subscription reversibility
- Independent signed update delivery
- Recovery from failed deployments or updates
