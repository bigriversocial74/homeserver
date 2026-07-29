# OpenRouter Model Provider v1

## Purpose

OpenRouter is an optional hosted-model adapter inside the HomeServer Model Center. HomeServer remains the agent harness, permission boundary, local data owner, MCP runtime, tool runtime, and receipt authority. OpenRouter supplies model inference only.

## Provider boundary

- Local Ollama remains available and is never removed.
- OpenRouter is disabled by default.
- No local content leaves HomeServer until the operator explicitly enables remote context and confirms `SEND REMOTE`.
- An explicitly selected local model never routes to OpenRouter.
- An explicitly selected `openrouter:<model-slug>` model uses OpenRouter and fails visibly if the remote request fails.
- When OpenRouter is the configured default and a remote request fails, HomeServer may continue with an available local model rather than sending content to another unapproved remote service.
- OpenRouter is not a VP3 licensing authority, Microgifter authority, updater trust root, MCP server, or agent runtime.

## Credential handling

The OpenRouter API key is stored under the existing `MicrogifterHomeServer` operating-system credential-vault service with the credential key:

`model-provider:openrouter:api-key`

The key is never written to SQLite, logs, receipts, frontend state, API responses, configuration files, backups, or GitHub.

## Local API

- `GET /v1/models/providers/openrouter`
- `GET /v1/models/providers/openrouter/catalog`
- `POST /v1/models/providers/openrouter/configure`
- `POST /v1/models/providers/openrouter/test`
- `POST /v1/models/providers/openrouter/disconnect`

All routes remain behind HomeServer's existing loopback-only secured router.

## Privacy and routing controls

The operator can configure:

- default model
- ordered fallback models
- monthly local dollar cap
- monthly local request cap
- maximum output tokens
- price, throughput, or latency routing priority
- provider fallback permission
- provider data-collection policy
- Zero Data Retention endpoint requirement
- whether Agent Workspace may send bounded prompt/evidence context remotely

OpenRouter requests use the fixed endpoint `https://openrouter.ai/api/v1`, reject redirects, use bounded request/response sizes, and never accept caller-controlled endpoints.

## Receipts

SQLite stores bounded usage receipts containing:

- provider and request IDs
- request kind
- requested and resolved model IDs
- token counts
- provider-reported cost when returned
- duration
- success/failure state and bounded error category

Receipts do not contain prompts, responses, Knowledge Vault content, operational evidence, conversation text, filenames, API keys, or credentials.

## Agent Workspace

Agent Workspace continues to assemble local, permission-scoped context. When OpenRouter is explicitly enabled for remote context, it sends only the same bounded prompt summary that would otherwise be sent to the local model endpoint. Imported operational evidence remains untrusted evidence, not instructions. Plans, approvals, execution, commerce actions, provider permissions, and receipts remain enforced by HomeServer and the connected authoritative platform.

## Database

Additive HomeServer SQLite migration:

`database/migrations/0016_openrouter_model_provider.sql`

No Microgifter or VP3 database migration is required for this provider adapter.

## Deployment

This feature requires a new certified HomeServer installer/update after merge. It does not require a reinstall when delivered through the existing signed update pipeline.
