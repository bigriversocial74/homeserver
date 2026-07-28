#!/usr/bin/env python3
from pathlib import Path

path = Path("scripts/smoke-test-service.ps1")
text = path.read_text(encoding="utf-8")
marker = '''    $workspace = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/agent/workspace" -TimeoutSec 15
'''
addition = '''    $reviewIntelligence = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/review-intelligence" -TimeoutSec 15
    if (-not $reviewIntelligence.deterministic_tracking_available -or -not $reviewIntelligence.llm_optional -or -not $reviewIntelligence.provider_authoritative -or -not $reviewIntelligence.imported_text_is_evidence_not_policy) {
        throw "Review Intelligence boundaries were not initialized"
    }
    if ($reviewIntelligence.settings.provider -ne "disabled" -or $reviewIntelligence.settings.remote_context_allowed -or $reviewIntelligence.settings.campaign_execution_enabled) {
        throw "Fresh Review Intelligence settings did not default to local disabled execution"
    }
    if ([int]$reviewIntelligence.observation_count -ne 0 -or [int]$reviewIntelligence.completed_runs -ne 0 -or @($reviewIntelligence.recent_clusters).Count -ne 0 -or @($reviewIntelligence.recommendations).Count -ne 0) {
        throw "Fresh Review Intelligence state is not empty"
    }

''' + marker
if "$apiBase/v1/review-intelligence" not in text:
    if marker not in text:
        raise SystemExit("Fresh Agent Workspace smoke anchor was not found")
    text = text.replace(marker, addition, 1)
path.write_text(text, encoding="utf-8", newline="\n")
print("Review Intelligence service smoke coverage applied to the fresh workspace check.")
