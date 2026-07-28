#!/usr/bin/env python3
from pathlib import Path

path = Path("src/review-intelligence.js")
text = path.read_text(encoding="utf-8")

old_fields = '''      <label>Campaign type<input id="review-plan-campaign-type" maxlength="80" value="${escapeHtml(draft.campaign_type || "customer_refund")}" required></label>
      <label>Microgifter campaign ID<input id="review-plan-campaign-id" maxlength="36" placeholder="Campaign UUID" required></label>
      <label>CRM contact ID <small>required for sends</small><input id="review-plan-contact-id" maxlength="36" placeholder="Contact UUID"></label>
'''
new_fields = '''      <label>Campaign type<input id="review-plan-campaign-type" maxlength="80" value="${escapeHtml(draft.campaign_type || "customer_refund")}" required></label>
      <label>Campaign title <small>required for a new draft</small><input id="review-plan-campaign-title" maxlength="180" value="${escapeHtml(draft.title || item.title)}"></label>
      <label>Microgifter campaign ID <small>leave blank to create a draft</small><input id="review-plan-campaign-id" maxlength="36" placeholder="Required for publish, pause, resume, or send"></label>
      <label>Reward template ID <small>optional for drafts</small><input id="review-plan-reward-id" maxlength="36" value="${escapeHtml(draft.reward_template_id || "")}" placeholder="Microgifter reward UUID"></label>
      <label>CRM contact ID <small>required for sends</small><input id="review-plan-contact-id" maxlength="36" placeholder="Contact UUID"></label>
'''
if old_fields in text:
    text = text.replace(old_fields, new_fields, 1)
elif new_fields not in text:
    raise SystemExit("campaign plan form fields anchor was not found")

old_variables = '''  const actionType = document.querySelector("#review-plan-action")?.value || "campaign.send_make_good";
  const campaignId = document.querySelector("#review-plan-campaign-id")?.value.trim() || "";
  const contactId = document.querySelector("#review-plan-contact-id")?.value.trim() || "";
  if (actionType.includes("send") && !contactId) {
'''
new_variables = '''  const actionType = document.querySelector("#review-plan-action")?.value || "campaign.send_make_good";
  const campaignId = document.querySelector("#review-plan-campaign-id")?.value.trim() || "";
  const campaignTitle = document.querySelector("#review-plan-campaign-title")?.value.trim() || "";
  const rewardTemplateId = document.querySelector("#review-plan-reward-id")?.value.trim() || "";
  const contactId = document.querySelector("#review-plan-contact-id")?.value.trim() || "";
  if (actionType !== "campaign.draft" && !campaignId) {
    notice = { kind: "warning", message: "A Microgifter campaign ID is required for publish, pause, resume, and send actions." };
    mount(true);
    return;
  }
  if (actionType === "campaign.draft" && !campaignTitle) {
    notice = { kind: "warning", message: "A title is required to create a real Microgifter campaign draft." };
    mount(true);
    return;
  }
  if (actionType.includes("send") && !contactId) {
'''
if old_variables in text:
    text = text.replace(old_variables, new_variables, 1)
elif new_variables not in text:
    raise SystemExit("campaign plan validation anchor was not found")

old_arguments = '''        campaign_type: document.querySelector("#review-plan-campaign-type")?.value.trim() || "customer_refund",
        campaign_id: campaignId,
        contact_id: contactId || null,
        channel: document.querySelector("#review-plan-channel")?.value || "microgifter_inbox",
        message: document.querySelector("#review-plan-message")?.value.trim() || "",
        evidence: item.evidence || {},
'''
new_arguments = '''        campaign_type: document.querySelector("#review-plan-campaign-type")?.value.trim() || "customer_refund",
        campaign_id: campaignId || null,
        title: campaignTitle || item.title,
        description: item.rationale,
        reward_template_id: rewardTemplateId || null,
        contact_id: contactId || null,
        channel: document.querySelector("#review-plan-channel")?.value || "microgifter_inbox",
        message: document.querySelector("#review-plan-message")?.value.trim() || "",
        evidence: item.evidence || {},
'''
if old_arguments in text:
    text = text.replace(old_arguments, new_arguments, 1)
elif new_arguments not in text:
    raise SystemExit("campaign plan arguments anchor was not found")

path.write_text(text, encoding="utf-8", newline="\n")
print("Review Intelligence now creates real provider campaign draft plans.")
