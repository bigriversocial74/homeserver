from pathlib import Path

root = Path(__file__).resolve().parents[1]
chat_path = root / "src" / "homeserver-agent-chat.js"
validator_path = root / "scripts" / "validate-agent-chat-route.py"

chat = chat_path.read_text(encoding="utf-8")
validator = validator_path.read_text(encoding="utf-8")

old = '''window.addEventListener("homeserver-shell-health", (event) => {
  shellHealth = { ...shellHealth, ...(event.detail || {}) };
  if (isAgentPage()) mount(true);
});'''
new = '''function applyShellHealth(detail) {
  shellHealth = { ...shellHealth, ...(detail || {}) };
  if (!isAgentPage()) return;
  const runtime = document.querySelector(".hs-runtime-state");
  if (!runtime) return;
  const serviceOffline = shellHealth.service === "offline";
  const modelsDegraded = shellHealth.models === "degraded";
  runtime.textContent = serviceOffline
    ? "HomeServer offline"
    : modelsDegraded
      ? "Model runtime offline"
      : humanize(workspace?.model_runtime_state || "ready");
  runtime.classList.toggle("warn", serviceOffline || modelsDegraded);
}

window.addEventListener("homeserver-shell-health", (event) => applyShellHealth(event.detail));'''
if old not in chat:
    raise SystemExit("Agent Chat health listener anchor not found")
chat = chat.replace(old, new, 1)

validator = validator.replace(
    '    \'window.addEventListener("homeserver-shell-health"\',\n',
    '    \'window.addEventListener("homeserver-shell-health"\',\n    \'function applyShellHealth(detail)\',\n    \'runtime.classList.toggle("warn"\',\n',
    1,
)
validator = validator.replace(
    "if 'event.target.closest(\"[data-page]\")' in main:\n",
    "if 'if (isAgentPage()) mount(true);' in chat:\n    raise SystemExit(\"Background shell health still remounts Agent Chat\")\nif 'event.target.closest(\"[data-page]\")' in main:\n",
    1,
)

chat_path.write_text(chat, encoding="utf-8")
validator_path.write_text(validator, encoding="utf-8")
print("Agent Chat shell health now updates the status badge without remounting the workspace.")
