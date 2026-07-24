# Microgifter HomeServer v1
## Reconciled Product and Technical Blueprint

**Status:** Authoritative approved product direction  
**Adopted:** 2026-07-24  
**Original approval:** `bigriversocial74/contactform` PR #1341  
**Original merge:** `80055acb325a6e5714f12ce9fd7d1283d20965a3`  
**Canonical implementation repository:** `bigriversocial74/homeserver`

## Product definition

Microgifter HomeServer is a private, locally installed extension of the Microgifter cloud platform. It gives merchants, creators, offices, studios, venues, hospitality businesses, community organizations, and other professional users local control over AI models, private business knowledge, automations, integrations, synchronized operational data, and approved agent access.

The standard customer edition is installed through one branded Windows executable:

`Microgifter-HomeServer-Setup.exe`

Customers must not need to install or configure Docker Desktop, PHP, Node.js, a database engine, a reverse proxy, or development tools.

HomeServer does not replace Microgifter.com. The cloud remains authoritative for identity, payments, shared commerce, campaigns, rewards, wallet ownership, PPPM ownership, claims, redemption, central permissions, and audit history.

## Locked product decisions

- Windows 11 x64 is the first customer platform.
- Windows EXE is the primary customer release.
- Tauri 2 is the Control Center shell.
- The standard release uses native Windows services.
- NSIS is the initial per-machine installer.
- Docker remains for development, Linux, NAS, cloud, and future appliances.
- Local installation may occur before cloud pairing.
- Public internet exposure is disabled by default.
- Large AI models are optional downloads.
- Read-only and low-risk local agents precede transactional automation.
- Backup, repair, update, restore, and uninstall are core product functions.
- HomeServer MCP cannot bypass Cloud MCP or Microgifter commerce rules.

## Product goals

HomeServer v1 must provide:

1. Branded one-click Windows installation.
2. Automatic background startup.
3. Desktop Control Center.
4. Secure Microgifter account pairing.
5. Local business knowledge and private files.
6. Optional local AI models.
7. Local MCP gateway for approved agents.
8. Scoped synchronization.
9. Selected offline operation.
10. Retry-safe outbound queues.
11. Automatic backup and guided recovery.
12. Signed and verified updates.
13. Customer-readable diagnostics and repair.
14. Clear cloud-versus-local authority.
15. A future Docker/Linux/NAS path without rewriting core contracts.

## Non-goals

HomeServer v1 is not a second payment processor, separate account system, replacement commerce lifecycle, public file server, smart-home platform, blockchain node, customer-managed Docker environment, or bypass around Microgifter approval and permission rules.

## Customer package

The Windows package includes:

- Control Center.
- Native service supervisor.
- Local API.
- Embedded operational database.
- Background workers and scheduler.
- Synchronization client.
- Secrets management.
- Knowledge Vault.
- MCP gateway.
- Backup and recovery service.
- Update manager.
- Health monitor.
- Diagnostics.
- Repair and uninstall support.

## Installer experience

Quick Install uses safe defaults: standard directories, automatic service startup, loopback-only access, automatic backups, stable update channel, deferred model download, and post-install cloud pairing.

Custom Install later supports custom data and backup locations, LAN access, proxy settings, update channel, optional model installation, and advanced ports.

Installation failures must identify the failed stage, provide a support code and redacted diagnostics, and offer retry, repair, or rollback.

## Control Center

Primary sections:

- Overview.
- Services.
- Microgifter Connection.
- Agents.
- MCP Access.
- Model Center.
- Knowledge Vault.
- Automations.
- Storage.
- Backups.
- Network.
- Users and Permissions.
- Updates.
- Logs and Support.
- Settings.

The Overview must show service health, cloud connection, synchronization state, backup health, storage, active model, update availability, and security warnings.

## Service architecture

The native service layer coordinates configuration, local authentication, API requests, local storage, background jobs, sync queues, AI routing, MCP requests, backups, updates, and health reporting.

Services start in dependency order, recover from controlled failures, avoid crash loops, coordinate updates and backups, and stop safely during repair or uninstall.

## Repository boundary

This repository owns local Windows software and packaging. `contactform` owns cloud APIs and authoritative commerce.

Any cloud dependency must be documented, authenticated, scoped, versioned, idempotent, audit-ready, and fail closed.

## Identity and pairing

Each installation receives a unique HomeServer ID, local key pair, server name, version, update channel, owner association, and cloud registration state.

Pairing requires explicit owner approval. Credentials are scoped and revocable. Local installation can finish before pairing, but cloud-dependent functions remain unavailable.

## Data authority

### Cloud authoritative

- User identity and account access.
- Merchant accounts and shared roles.
- Payments and purchases.
- Public campaigns and reward inventory.
- Wallet and PPPM ownership.
- Microgift lifecycle.
- Claims and redemption.
- Shared permissions and central audit history.

### HomeServer authoritative

- Local configuration and encryption keys.
- Installed models and local model preferences.
- Private business documents and indexes.
- Local device approvals.
- Local integration credentials.
- Local automations.
- Local-only agent configuration.
- Backup history and diagnostics.
- Unsynchronized local work before cloud acceptance.

### Synchronized copies

Merchant profiles, approved CRM data, product and campaign summaries, analytics, engagement history, settings, and reporting datasets may be cached locally without becoming authoritative.

## Synchronization

Synchronization must be authenticated, encrypted, scoped, idempotent, auditable, retry-safe, version-aware, and conflict-aware.

Cloud-authoritative conflicts resolve to cloud state. Local-authoritative conflicts resolve to local state. High-risk and commerce conflicts require explicit review and cannot silently overwrite cloud records.

Queued commerce work must never be displayed as completed before cloud acceptance.

## Offline operation

Available offline:

- Control Center.
- Local authentication.
- Local knowledge and search.
- Local model inference.
- Approved local automations.
- Reports from cached data.
- Backup, recovery, and service management.
- Queueing eligible outbound requests.

Deferred offline:

- Payments.
- Final commerce transactions.
- Public publishing.
- Ownership changes.
- Claims and redemption requiring current cloud validation.
- Actions requiring live inventory, permission, or approval state.

## MCP and agents

Cloud MCP remains authoritative for centralized Microgifter commerce. HomeServer MCP provides controlled access to local documents, models, integrations, automations, cached business context, and approved cloud tools.

Every agent is subject to authentication, ownership, permission scopes, campaign and reward rules, budget limits, approvals, rate limits, idempotency, current cloud state, and audit receipts.

Capability progression:

1. Read-only knowledge and reporting.
2. Low-risk local actions.
3. Approval-gated cloud action requests.
4. Narrow policy-authorized automation.

## Model Center

Ollama is the first recommended local runtime. Model Center detects hardware, recommends compatible models, manages downloads and removal, tests models, assigns models to agents, controls CPU/RAM/GPU usage, and exposes cloud fallback only when explicitly allowed.

Models receive no file, tool, integration, or cloud access without explicit scopes.

## Knowledge Vault

Knowledge Vault indexes approved documents, policies, menus, scripts, marketing assets, procedures, training content, and other local business files.

It supports folder selection, metadata, tags, file-change detection, duplicate handling, search, retention, deletion, backup inclusion, and per-agent access rules.

## Security

Required controls include code signing, signed updates, secure pairing, encrypted transport and secrets, strong local authentication, session expiry, role-based access, local API authentication, rate limiting, audit and security logs, encrypted backups, credential rotation, and redacted diagnostics.

The local API binds to loopback by default. LAN access requires explicit owner approval, device authorization, firewall configuration, transport security, visible device lists, and revocation.

No direct public port forwarding is recommended or enabled.

## Backup, recovery, updates, repair, and uninstall

HomeServer provides daily encrypted backups, pre-update backups, integrity checks, retention policies, external storage options, encrypted recovery packages, guided restore, signed update packages, migration checks, health verification, rollback where safe, repair installation, and preserve-data uninstall as the default.

Permanent local data deletion requires explicit confirmation.

## Release phases

1. Installable foundation.
2. Cloud pairing and synchronization.
3. Backup, recovery, and updates.
4. Knowledge Vault and Model Center.
5. MCP and agent runtime.
6. Advanced deployment, secure remote access, Linux/NAS, and appliance preparation.

## Phase 1 completion standard

Phase 1 requires a buildable Tauri Control Center, native Windows service, local API, local database, health model, logging, NSIS installer path, repair/uninstall foundation, and CI evidence. It does not claim the full v1 product is production-ready.

## Authoritative statement

Microgifter HomeServer is a private local extension of Microgifter. It provides local AI, business knowledge, automations, integrations, synchronized data, and secure agent access while preserving Microgifter’s centralized identity, commerce, campaign, reward, wallet, PPPM, claim, redemption, permission, and audit rules.

The customer installs Microgifter, not an infrastructure stack.
