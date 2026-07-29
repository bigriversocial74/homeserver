# Native POD Provider Voice Adapter v1

Contract: `pod-homeserver-voice-1`

Repository: `bigriversocial74/homeserver`

## Purpose

The native POD provider adapter lets an independently owned HomeServer pair with one or more authorized POD wrappers and process bounded voice jobs locally. It sits beside the existing Microgifter provider and reuses HomeServer's provider-neutral connection registry, operating-system credential vault, local API, service lifecycle, installer, backup, and audit foundations.

The adapter does not make a POD an updater trust root and does not give a POD access to Knowledge Vault documents, private conversations, prompts, models, operational datasets, review intelligence, other wrapper connections, or unrelated provider credentials.

## Connection flow

1. The POD owner issues a short-lived one-time Sync Code.
2. The HomeServer owner opens Cloud & POD Connection Registry.
3. The owner selects POD Wrapper, enters the canonical HTTPS POD URL, connection name, and Sync Code.
4. HomeServer generates an Ed25519 signing key locally.
5. HomeServer exchanges the Sync Code once through `/api/homeserver/v1/pairing/exchange`.
6. The returned bearer credential and Ed25519 seed are stored only in the operating-system credential vault.
7. SQLite stores provider identity, connection state, capability grants, credential references, hashes, hints, jobs, and receipts.
8. The background worker sends signed heartbeats and polls the exact paired POD for jobs.

## Signed request contract

Every post-pairing request includes:

- Bearer credential
- HomeServer device UUID
- POD provider connection UUID
- Unix timestamp
- Unique nonce
- Ed25519 signature
- HomeServer version

Canonical signature input:

```text
METHOD
PATH
TIMESTAMP
NONCE
SHA256_HEX(RAW_BODY)
```

Redirects are disabled. Production provider URLs require HTTPS; HTTP is permitted only for loopback tests. Query-string credentials are not used.

## Capabilities

The adapter recognizes only:

```text
pod.pairing.v1
pod.device-heartbeat.v1
pod.voice.jobs.v1
pod.voice.transcription.v1
pod.voice.synthesis.v1
pod.voice.artifacts.v1
pod.voice.receipts.v1
pod.receptionist.context.v1
```

Unknown capabilities are not activated. A leased job is accepted only when its required capability is granted to that exact connection.

## Local voice runtime

Each paired POD connection has independent runtime settings:

- Speech-to-text enabled state
- Absolute transcription executable path
- Bounded argument list
- Transcription model label
- Text-to-speech enabled state
- Absolute synthesis executable path
- Bounded argument list
- Synthesis model and optional voice
- Execution timeout
- Maximum input and output bytes

Commands are launched with `Command::new` and argument arrays. No command shell is invoked. Executable paths must be absolute existing files. Runtime work is placed beneath the HomeServer data directory and removed after the job.

### Speech-to-text command contract

HomeServer appends:

```text
--input <local-audio-path>
--output <result-json-path>
--job-id <remote-job-uuid>
--language <language>
```

Expected result JSON:

```json
{
  "transcript": "Bounded transcript text",
  "language": "en-US",
  "confidence": 0.95,
  "model": "local-model",
  "processing_ms": 1200
}
```

### Text-to-speech command contract

HomeServer appends:

```text
--input-json <request-json-path>
--output-audio <audio-path>
--output-json <result-json-path>
--job-id <remote-job-uuid>
```

Expected result JSON:

```json
{
  "audio_path": "<path beneath the assigned work directory>",
  "mime_type": "audio/mpeg",
  "model": "local-model",
  "processing_ms": 900
}
```

Supported output MIME types are MP3, WAV, OGG, and WebM. Returned paths must remain beneath the assigned job directory.

## Job lifecycle

1. HomeServer polls the paired POD.
2. The POD atomically leases one job and returns a one-time lease token.
3. HomeServer stores the lease token only in the operating-system credential vault.
4. The adapter validates job UUID, type, capability, payload hash, lease format, bounds, and artifact metadata.
5. Input audio is fetched through the signed artifact endpoint and verified by hash and byte count.
6. The configured local runtime executes without a shell.
7. HomeServer submits a successful result or a bounded stable failure code.
8. Local job state and receipts are recorded.
9. Lease credentials and temporary work files are removed.

Retryable runtime timeouts and generic runtime failures may return to the POD queue while attempts remain. Configuration, artifact, size, and unsupported-job failures are terminal.

## Failure categories

```text
pod_runtime_unconfigured
pod_runtime_timeout
pod_artifact_invalid
pod_runtime_output_too_large
pod_runtime_failed
pod_worker_cycle_failed
connection_disconnected
```

## Independence boundaries

- HomeServer operates locally with zero POD connections.
- Browser voice remains available when no local runtime is configured.
- Human WebRTC calling remains separate.
- Microgifter synchronization and entitlement remain separate.
- Signed update authorization remains separate.
- Each POD connection has its own credential, capabilities, jobs, history, and revocation state.
- Disconnecting one POD does not affect other providers or wrappers.
