# POD Provider Voice Adapter Setup v1

## Section score

- Initial audit: **4.7/10**
- Certified target: **10/10**

## Delivered

- Native `pod` provider beside Microgifter
- One-time POD Sync Code pairing
- Local Ed25519 signing identity
- Operating-system credential-vault storage for bearer credentials, signing seeds, and job leases
- Signed heartbeat, poll, artifact-read, completion, and failure requests
- Explicit capability negotiation
- Per-connection isolation and revocation
- Pull-based capability, speech-to-text, and text-to-speech jobs
- Absolute-path, no-shell local runtime execution
- Audio/result hash and byte-count verification
- Job attempts, expiry, retry, history, and bounded receipts
- Control Center pairing, runtime configuration, manual polling, state, and disconnect controls
- Permanent regression in `npm run check:frontend`
- Browser voice and local-only HomeServer operation preserved

## Installation

This feature is delivered through the normal verified HomeServer installer/update package. Do not copy individual service binaries into a live installation.

1. Back up HomeServer.
2. Install the verified Section 7 HomeServer package.
3. Confirm the HomeServer service and Control Center open normally.
4. Open **Cloud & POD Connection Registry**.
5. Keep the local voice runtimes disabled until valid executable paths are available.
6. On the POD, import and deploy the POD provider foundation through v63.5.
7. Configure the POD's private bridge secret and enable its provider endpoint.
8. Issue a one-time POD Sync Code.
9. In HomeServer, select **POD Wrapper** and enter:
   - Connection name
   - Canonical HTTPS POD URL
   - One-time POD Sync Code
10. Confirm the connection shows its POD identity, device UUID, granted capabilities, heartbeat, and poll state.
11. Configure local speech-to-text and/or text-to-speech executables.
12. Save the runtime settings and confirm the runtime state becomes Ready.
13. Queue capability, transcription, and synthesis tests from the POD owner console.
14. Use **Poll Now** or wait for the background worker.
15. Confirm results and receipts appear on both sides.

## Runtime executable requirements

- Absolute path only
- Existing local file
- No shell wrapper
- No interactive prompt
- Deterministic exit status
- Writes bounded JSON to the assigned output path
- Writes audio only beneath the assigned job work directory
- Does not require POD credentials as command-line arguments
- Does not access unrelated HomeServer data

## Speech-to-text arguments added by HomeServer

```text
--input <path>
--output <path>
--job-id <uuid>
--language <language>
```

## Text-to-speech arguments added by HomeServer

```text
--input-json <path>
--output-audio <path>
--output-json <path>
--job-id <uuid>
```

Additional configured arguments are placed before these adapter-owned arguments.

## Security verification

- Raw POD bearer credential is absent from SQLite.
- Ed25519 private seed is absent from SQLite.
- Remote job lease token is absent from SQLite.
- Sync Code is never stored by HomeServer after pairing.
- Provider redirects are rejected.
- Production provider URL requires HTTPS.
- Job payload hash must match.
- Artifact UUID, MIME, hash, and byte count must match.
- Output audio remains within configured limits.
- Runtime result JSON remains bounded.
- Runtime command uses no shell.
- Disconnect removes the POD credential and active lease entries.
- Disconnect leaves Microgifter, MCP, Knowledge Vault, models, backups, updater, and local operation intact.

## Rollback

The migration is additive. Rolling back the application does not require dropping tables. Disable/disconnect POD connections first, then install the previous verified HomeServer package. Preserve the HomeServer data directory.
