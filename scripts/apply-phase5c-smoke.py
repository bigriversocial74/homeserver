#!/usr/bin/env python3
"""Add permanent Phase 5C operational data checks to the service smoke test."""
from pathlib import Path

path = Path(__file__).resolve().parents[1] / "scripts/smoke-test-service.ps1"
value = path.read_text(encoding="utf-8")

if '"operational_data_evidence" -notin @($workspace.capabilities)' in value:
    print("Phase 5C operational data smoke coverage is already applied.")
    raise SystemExit(0)


def replace_once(old: str, new: str, label: str) -> None:
    global value
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, found {count}")
    value = value.replace(old, new, 1)


replace_once(
    '''    $workspace = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/agent/workspace" -TimeoutSec 15
''',
    '''    $operational = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/operational-data" -TimeoutSec 15
    if (-not $operational.local_only -or -not $operational.provider_authoritative -or -not $operational.imported_data_is_untrusted_evidence) {
        throw "Operational data authority or local evidence boundary was not initialized"
    }
    if ([int]$operational.provider_manifests -ne 1 -or [int]$operational.enabled_grants -ne 0 -or [int]$operational.imported_records -ne 0 -or [int]$operational.imported_events -ne 0) {
        throw "Fresh operational data manifest or evidence state is invalid"
    }
    if (@($operational.datasets).Count -ne 0 -or @($operational.recent_runs).Count -ne 0) {
        throw "Fresh HomeServer unexpectedly exposes connection datasets or import history"
    }
    $emptyOperationalQueryBody = @{ connection_id = $null; dataset_key = $null; source_object_type = $null; limit = 25 } | ConvertTo-Json -Compress
    $emptyOperationalQuery = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/operational-data/query" -ContentType "application/json" -Body $emptyOperationalQueryBody -TimeoutSec 15
    if ([int]$emptyOperationalQuery.available_records -ne 0 -or @($emptyOperationalQuery.records).Count -ne 0 -or -not $emptyOperationalQuery.provider_authoritative -or -not $emptyOperationalQuery.imported_data_is_untrusted_evidence) {
        throw "Fresh operational evidence query did not preserve its empty provider-authoritative boundary"
    }
    $invalidOperationalImportBody = @{
        connection_id = [guid]::NewGuid().ToString()
        provider_key = "microgifter"
        tenant_id = "wrong-tenant"
        site_id = "wrong-site"
        dataset_key = "merchant.products"
        import_mode = "snapshot"
        cursor_after = $null
        source_revision = "ci-invalid"
        records = @(@{ source_object_type = "product"; source_object_id = "ci-invalid"; source_revision = "1"; source_updated_at_utc = $null; payload = @{ name = "Rejected evidence" } })
        events = @()
    } | ConvertTo-Json -Depth 10 -Compress
    $invalidOperationalImport = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/operational-data/import" -ContentType "application/json" -Body $invalidOperationalImportBody -TimeoutSec 15
    if ($invalidOperationalImport.StatusCode -ne 422) {
        throw "Expected unknown-connection operational import rejection, received HTTP $($invalidOperationalImport.StatusCode)"
    }

    $workspace = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/agent/workspace" -TimeoutSec 15
''',
    "operational smoke block",
)
replace_once(
    '''    if ("approval_gated_execute" -notin @($workspace.capabilities)) {
        throw "Agent Workspace is missing its supervised execution capability marker"
    }
''',
    '''    if ("approval_gated_execute" -notin @($workspace.capabilities) -or "operational_data_evidence" -notin @($workspace.capabilities)) {
        throw "Agent Workspace is missing supervised execution or operational evidence capability markers"
    }
''',
    "workspace capability",
)
replace_once(
    '''    if (-not $operationalSource -or $operationalSource.state -ne "planned_phase_5c" -or -not $worldSource -or $worldSource.state -ne "mission_drafting") {
        throw "Agent Workspace did not expose the Phase 5C and World Mission boundaries"
    }
''',
    '''    if (-not $operationalSource -or $operationalSource.state -ne "empty" -or -not $worldSource -or $worldSource.state -ne "mission_drafting") {
        throw "Agent Workspace did not expose the operational evidence and World Mission boundaries"
    }
''',
    "workspace operational state",
)
replace_once(
    '    Write-Host "HomeServer encrypted backup, exported recovery, fresh-install import, verification, staged restore, and rollback-ready smoke test passed."\n',
    '    Write-Host "HomeServer operational evidence, supervised agents, MCP, encrypted backup, exported recovery, fresh-install import, verification, staged restore, and rollback-ready smoke test passed."\n',
    "smoke completion message",
)

path.write_text(value, encoding="utf-8", newline="\n")
print("Phase 5C operational data smoke coverage applied.")
