from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    source = target.read_text(encoding="utf-8")
    if old not in source:
        if new in source:
            return
        raise SystemExit(f"Expected source anchor not found in {path}")
    target.write_text(source.replace(old, new, 1), encoding="utf-8")


replace_once(
    "src/homeserver-agent-chat.js",
    '      <div class="hs-chat-sidebar-brand"><span>✦</span><div><strong>HomeServer</strong><small>Private Agent</small></div></div>',
    '      <button class="hs-chat-sidebar-brand" id="hs-chat-logo-home" type="button" aria-label="Back to dashboard" title="Back to dashboard"><span>✦</span><div><strong>HomeServer</strong><small>Private Agent</small></div></button>',
)
replace_once(
    "src/homeserver-agent-chat.js",
    '  document.querySelector("#hs-chat-control-center")?.addEventListener("click", () => { window.location.hash = "#dashboard"; });',
    '  document.querySelectorAll("#hs-chat-logo-home,#hs-chat-control-center").forEach((button) => {\n    button.addEventListener("click", () => { window.location.hash = "#dashboard"; });\n  });',
)
replace_once(
    "src/homeserver-agent-chat.css",
    '.hs-chat-sidebar-brand{display:flex;align-items:center;gap:10px;padding:0 6px 18px}',
    '.hs-chat-sidebar-brand{display:flex;align-items:center;gap:10px;width:100%;padding:0 6px 18px;border:0;background:transparent;color:inherit;text-align:left;font:inherit;cursor:pointer}.hs-chat-sidebar-brand:hover strong{color:#2563eb}.hs-chat-sidebar-brand:focus-visible{outline:2px solid #2563eb;outline-offset:2px;border-radius:12px}',
)
replace_once(
    "scripts/validate-agent-chat-route.py",
    "    'id=\"hs-chat-control-center\"',\n",
    "    'id=\"hs-chat-control-center\"',\n    'id=\"hs-chat-logo-home\"',\n",
)

print("HomeServer Agent logo now returns to the dashboard.")
