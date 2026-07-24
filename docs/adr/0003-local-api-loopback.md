# ADR 0003: Loopback-only local API

- Status: Accepted
- Date: 2026-07-24

## Decision

The Phase 1 local API binds only to `127.0.0.1:47831`.

The Control Center uses the API for service health and local administration. LAN and remote access remain disabled until explicit device authorization, TLS, firewall, and revocation controls are implemented.

## Security consequence

No router port forwarding, public listener, database listener, model listener, or MCP listener is enabled by Phase 1.
