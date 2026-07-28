#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, value: str) -> None:
    (ROOT / path).write_text(value, encoding="utf-8", newline="\n")


def replace_once(value: str, old: str, new: str, label: str) -> str:
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, found {count}")
    return value.replace(old, new, 1)

lib = read("src-tauri/src/lib.rs")
if "mod review_intelligence;" not in lib:
    lib = replace_once(lib, "mod operational;\n", "mod operational;\nmod review_intelligence;\n", "Tauri review module")
if "review_intelligence::homeserver_review_intelligence" not in lib:
    lib = replace_once(
        lib,
        """            operational::homeserver_query_operational_data,
""",
        """            operational::homeserver_query_operational_data,
            review_intelligence::homeserver_review_intelligence,
            review_intelligence::homeserver_update_review_intelligence_settings,
            review_intelligence::homeserver_sync_review_dataset,
            review_intelligence::homeserver_run_review_analysis,
            review_intelligence::homeserver_record_review_recommendation_outcome,
""",
        "Tauri review commands",
    )
write("src-tauri/src/lib.rs", lib)

index = read("index.html")
if "/src/review-intelligence.js" not in index:
    index = replace_once(
        index,
        '    <script type="module" src="/src/operational-data.js"></script>\n',
        '    <script type="module" src="/src/operational-data.js"></script>\n    <script type="module" src="/src/review-intelligence.js"></script>\n',
        "Review Intelligence frontend module",
    )
write("index.html", index)

operational = read("src/operational-data.js")
operational = operational.replace(
    '<div class="operational-footnote">Detailed payment data, private messages, gift ownership, and full customer contact records are not present in the Phase 5C-A manifest.</div>',
    '<div class="operational-footnote">Reviews, messages, CRM contact details, purchase history, and gift ownership can be authorized as restricted or sensitive evidence. Raw card numbers, CVV/CVC, private keys, API secrets, reusable payment credentials, and processor secrets are never part of the operational intelligence layer.</div>',
)
old_checks = '''<label><input type="checkbox" name="operational-agent-use" value="read" ${uses.has("read") ? "checked" : ""}>Read evidence</label><label><input type="checkbox" name="operational-agent-use" value="analyze" ${uses.has("analyze") ? "checked" : ""}>Analyze</label><label><input type="checkbox" name="operational-agent-use" value="goal_match" ${uses.has("goal_match") ? "checked" : ""}>Match goals</label><label><input type="checkbox" name="operational-agent-use" value="report" ${uses.has("report") ? "checked" : ""}>Create reports</label>'''
new_checks = old_checks + '''<label><input type="checkbox" name="operational-agent-use" value="sentiment_analysis" ${uses.has("sentiment_analysis") ? "checked" : ""}>Analyze sentiment</label><label><input type="checkbox" name="operational-agent-use" value="semantic_clustering" ${uses.has("semantic_clustering") ? "checked" : ""}>Group recurring context</label><label><input type="checkbox" name="operational-agent-use" value="conversation_continuity" ${uses.has("conversation_continuity") ? "checked" : ""}>Maintain conversation context</label><label><input type="checkbox" name="operational-agent-use" value="service_recovery" ${uses.has("service_recovery") ? "checked" : ""}>Recommend service recovery</label><label><input type="checkbox" name="operational-agent-use" value="campaign_management" ${uses.has("campaign_management") ? "checked" : ""}>Prepare campaign actions</label><label><input type="checkbox" name="operational-agent-use" value="consent_enforcement" ${uses.has("consent_enforcement") ? "checked" : ""}>Use consent evidence</label>'''
if old_checks in operational and "value=\"sentiment_analysis\"" not in operational:
    operational = operational.replace(old_checks, new_checks, 1)
write("src/operational-data.js", operational)

review = read("src/review-intelligence.js")
old = '''  document.querySelector("#review-provider")?.addEventListener("change", () => mount(true));
'''
new = '''  document.querySelector("#review-provider")?.addEventListener("change", (event) => {
    snapshot.settings.provider = event.target.value;
    mount(true);
  });
'''
if old in review:
    review = review.replace(old, new, 1)
elif new not in review:
    raise SystemExit("Review provider change anchor was not found")
write("src/review-intelligence.js", review)

print("Review Intelligence Tauri and frontend integration applied.")
