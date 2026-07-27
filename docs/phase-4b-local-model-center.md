# Phase 4B — Local Model Center Foundation

Phase 4B adds the first production-bounded local model-management layer to Microgifter HomeServer.

## Runtime boundary

- Ollama is the only supported runtime in this phase.
- HomeServer connects only to `http://127.0.0.1:11434`.
- The runtime URL is not user-configurable.
- Redirects are disabled.
- There is no cloud prompt fallback, remote runtime, LAN listener, MCP tool execution, or autonomous agent runtime.
- Knowledge Vault content is not sent to Ollama automatically. Semantic indexing remains a later Phase 4 scope.

## Delivered controls

- Runtime and version detection.
- Local installed-model and loaded-model inventory.
- CPU, memory, disk, and optional deployment-provided GPU metadata.
- Hardware-aware recommendations from a small approved starter catalog.
- Bounded, resumable Ollama pulls with restart-safe SQLite operation records.
- Duplicate active-download prevention.
- Explicit local model deletion.
- Model unload through Ollama keep-alive controls.
- Bounded local chat and embedding tests.
- Default chat and embedding model assignments.
- Context, test-timeout, and maximum-download limits.
- Control Center runtime, inventory, catalog, operation, test, and settings views.

## Ollama installation assistant

When Ollama is unavailable, Model Center now presents a Windows setup assistant that:

- Opens only the fixed official Ollama installer and documentation URLs through the Windows default browser.
- Displays and copies Ollama's official PowerShell installation command.
- Opens a normal PowerShell window with instructions, but never executes the remote installation script automatically.
- Provides CLI and loopback API verification commands.
- Rechecks the existing HomeServer runtime endpoint after installation.
- Shows repair, update, uninstall, and model-storage guidance.

The setup assistant does not bundle, silently install, elevate, update, or uninstall Ollama. The user remains in control of the official Ollama installer.

## Approved starter catalog

- `gemma3:1b`
- `llama3.2:1b`
- `llama3.2:3b`
- `gemma3:4b`
- `nomic-embed-text:latest`

Arbitrary model identifiers are rejected by the HomeServer service. Catalog changes require a reviewed HomeServer release.

## Failure behavior

- HomeServer remains healthy when Ollama is not installed or is stopped.
- Model actions return a bounded local runtime error when Ollama is unavailable.
- Active pull records become `interrupted` after a HomeServer restart.
- Ollama can resume interrupted layer downloads when the approved pull is started again.
- Pull progress responses and local API responses have explicit size limits.
- Model deletion requires the literal confirmation `DELETE`.
- If the default browser or PowerShell cannot be opened, the setup assistant preserves copyable official URLs and commands.

## Deferred work

- Bundled, silent, or enterprise-managed Ollama installation and update management.
- Automatic GPU driver/runtime management.
- PDF/OCR extraction.
- Semantic Knowledge Vault indexing.
- MCP server and permission-controlled agent runtime.
- Cloud model execution or cloud prompt fallback.
