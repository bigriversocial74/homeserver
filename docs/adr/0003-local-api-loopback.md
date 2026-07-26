# ADR 0003: Loopback-only authenticated local API

- Status: Accepted and hardened
- Date: 2026-07-24
- Hardened: 2026-07-25

## Decision

The HomeServer local API binds only to `127.0.0.1:47831`.

The native Control Center uses this API for service health and local administration. LAN and remote access remain disabled until explicit device authorization, TLS, firewall, and revocation controls are implemented.

Loopback binding is necessary but is not treated as sufficient authorization. The complete merged router applies these controls:

- Requests carrying browser `Origin` or Fetch Metadata headers are rejected.
- `POST`, `PUT`, `PATCH`, and `DELETE` requests require the trusted `X-MG-Local-Client` Control Center marker.
- Responses are `no-store`, `nosniff`, frame-denied, no-referrer, and same-origin resource protected.
- Request body sizes are bounded by route class.
- The service is not configured for CORS.

The trusted marker prevents drive-by browser requests; it is not represented as a cryptographic user-authentication mechanism. Future remote/LAN access requires a separate authenticated protocol and must not weaken this loopback boundary.

## Security consequence

No router port forwarding, public listener, database listener, model listener, or MCP listener is enabled by the current release. Cloud pairing, synchronization, backup, recovery, and update mutations all pass through the same local request boundary.
