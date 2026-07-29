# Native POD Provider Voice Adapter v1 Scorecard

| Area | Before | After | Certification |
|---|---:|---:|---|
| Existing provider-neutral HomeServer foundation | 1.0/1.0 | 1.0/1.0 | Existing registry, keyring, local API, installer, backup, and updater retained |
| POD pairing and permanent connection identity | 0.3/1.0 | 1.0/1.0 | Sync Code, local Ed25519 identity, provider/device UUIDs |
| Signed authentication and credential protection | 0.5/1.25 | 1.25/1.25 | Bearer + Ed25519, no redirects, keyring-only raw secrets |
| Capability and connection isolation | 0.6/1.0 | 1.0/1.0 | Explicit POD capabilities and per-connection enforcement |
| Heartbeat and pull-based job worker | 0.4/1.0 | 1.0/1.0 | Background and manual heartbeat/poll lifecycle |
| Local STT/TTS runtime execution | 0.4/1.5 | 1.5/1.5 | Absolute executables, argv-only execution, bounded contracts |
| Artifact and result integrity | 0.4/1.0 | 1.0/1.0 | UUID, MIME, SHA-256, bytes, output-path, and size verification |
| Retry, receipts, history, and disconnect | 0.4/0.75 | 0.75/0.75 | Attempts, stable failures, retention, lease/credential cleanup |
| Control Center owner experience | 0.3/0.75 | 0.75/0.75 | Pairing, status, runtime settings, polling, disconnect |
| Compatibility and authority boundaries | 0.4/0.75 | 0.75/0.75 | Microgifter, updater, MCP, vault, browser voice, and local operation independent |
| **Total** | **4.7/10** | **10/10** | **Certified by HomeServer Production Quality run 30409994248 and Coordinated Cloud Connector Contract run 30409994226** |
