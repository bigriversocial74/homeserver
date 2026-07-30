# VP3 Network Client v1

The HomeServer service now consumes the VP3 software-authority control plane independently of Microgifter provider pairing.

## Local activation

1. Read `GET /v1/software-authority/fingerprint`.
2. Register that SHA-256 fingerprint against an eligible HomeServer license in VP3.
3. Submit the one-time VP3 device credential and enrollment code to `POST /v1/software-authority/activate` with confirmation `ACTIVATE VP3`.
4. HomeServer verifies the Ed25519 entitlement lease using its pinned authority public key, stores the device credential in the operating-system credential vault, and cuts over atomically.

## Runtime

- heartbeat every five minutes
- signed lease refresh before expiration
- fail-closed update eligibility
- signed VP3 release document verification
- device/release/artifact-scoped installer grants stored only in the credential vault
- SHA-256, byte-size, and Authenticode verification
- durable idempotent update-receipt outbox
- no Microgifter authority dependency

The client trusts only HTTPS endpoints on `vp3.me` or its subdomains, rejects redirects, and never accepts a caller-provided installer host.
