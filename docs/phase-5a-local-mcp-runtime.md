# Phase 5A — Local Read-only MCP Runtime

## Status

Phase 5A adds a local Model Context Protocol runtime to Microgifter HomeServer. It is intended for approved agent harnesses running on the same Windows computer and exposes only client-scoped, read-only local capabilities.

The runtime does not expose a LAN or public listener, does not provide arbitrary file-system access, and does not authorize agents to modify HomeServer, Microgifter Cloud, commerce, campaigns, rewards, claims, redemption, models, documents, settings, or operating-system state.

## Transports

HomeServer provides:

- Streamable HTTP at `http://127.0.0.1:47831/mcp`.
- A packaged `microgifter-homeserver-mcp.exe` stdio bridge for desktop agent harnesses.

The HTTP endpoint is inside the existing HomeServer loopback API and inherits the fixed-host, anti-browser, anti-forwarding, no-store, and security-header boundary. The stdio bridge has a fixed endpoint and accepts its client token only through `MG_HOMESERVER_MCP_TOKEN`. It never accepts an arbitrary URL.

Supported protocol revisions:

- `2025-11-25`
- `2025-06-18`
- `2025-03-26`

## Read-only capabilities

Client scopes control which tools and resources are visible:

- `system.read` — HomeServer service health and pending-work status.
- `cloud.read` — pairing and signed synchronization status without credentials.
- `models.read` — approved local Ollama runtime and installed-model inventory.
- `knowledge.search` — cited keyword, semantic, or hybrid Knowledge Vault retrieval.
- `knowledge.read` — approved document metadata and a bounded indexed-text view.

Every advertised tool is marked read-only, non-destructive, idempotent, and closed-world.

## Client credentials

MCP clients are created explicitly from the Control Center. HomeServer:

1. Generates a random `mghs_mcp_...` bearer token.
2. Displays the raw token once.
3. Stores only its SHA-256 hash and a short non-secret hint.
4. Applies selected scopes and an expiry of 1 to 365 days.
5. Allows immediate revocation from the Control Center.

A maximum of 100 active clients is allowed. Each client is limited to 120 authenticated requests per minute.

## Bounds and auditing

- 128 KB maximum JSON-RPC request.
- 1 MB maximum JSON-RPC response.
- 200-character Knowledge Vault query.
- 20 search results.
- 20,000 indexed characters per document response.
- 200 document catalog records.
- 90-day and 5,000-record audit receipt retention.

Audit receipts include client ID, method, capability, outcome, bounded request/response sizes, duration, and timestamp. They never store tokens, prompts, queries, document text, model output, cloud credentials, or file content.

## Example Streamable HTTP configuration

```json
{
  "mcpServers": {
    "homeserver": {
      "url": "http://127.0.0.1:47831/mcp",
      "headers": {
        "Authorization": "Bearer mghs_mcp_COPY_THE_ONE_TIME_TOKEN"
      }
    }
  }
}
```

## Example stdio configuration

```json
{
  "mcpServers": {
    "homeserver": {
      "command": "C:\\Program Files\\Microgifter HomeServer\\resources\\microgifter-homeserver-mcp.exe",
      "env": {
        "MG_HOMESERVER_MCP_TOKEN": "mghs_mcp_COPY_THE_ONE_TIME_TOKEN"
      }
    }
  }
}
```

## Deferred to Phase 5B

Phase 5A does not execute local actions, call cloud tools, run autonomous workflows, or accept policy-authorized writes. Phase 5B adds supervised agents with explicit plans, approval requests, bounded local actions, and auditable receipts. Cloud actions remain approval-gated and subject to current Microgifter permissions and business rules.
