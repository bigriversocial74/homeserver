# OpenRouter API Contract

The HomeServer integration uses the fixed OpenRouter API base `https://openrouter.ai/api/v1`.

- Model discovery: `GET /models/user`
- Chat inference: `POST /chat/completions`
- Authentication: bearer API key from the local operating-system credential vault
- Optional application headers: `HTTP-Referer: https://vp3.me` and `X-Title: VP3 HomeServer`
- Redirects: rejected
- Request and response sizes: bounded
- Provider routing controls: sort, provider fallback permission, data-collection policy, and optional ZDR-only routing
- Model fallbacks: configured locally and sent only when explicitly enabled

OpenRouter does not receive HomeServer license credentials, Microgifter provider credentials, MCP client tokens, private keys, payment credentials, or unrelated wrapper data.
