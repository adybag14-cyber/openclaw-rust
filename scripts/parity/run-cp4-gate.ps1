param(
  [string]$CargoCommand = "cargo",
  [string]$Toolchain = "1.83.0-x86_64-pc-windows-gnu",
  [string]$ArtifactDir = "parity/generated/cp4"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$tests = @(
  "channels::tests::exposes_channel_capabilities_and_wave1_order",
  "channels::tests::signal_driver_detects_source",
  "channels::tests::webchat_driver_detects_source",
  "channels::tests::normalize_chat_type_supports_dm_alias",
  "channels::tests::mention_gate_skips_when_required_and_not_mentioned",
  "channels::tests::mention_gate_with_bypass_allows_authorized_control_commands",
  "channels::tests::chunking_supports_length_and_newline_modes",
  "channels::tests::default_chunk_limit_matches_core_channel_defaults",
  "channels::tests::retry_backoff_policy_scales_and_caps",
  "scheduler::tests::mention_activation_accepts_group_message_when_detection_unavailable",
  "scheduler::tests::mention_activation_bypasses_for_authorized_control_command",
  "gateway::tests::dispatcher_channels_methods_report_status_and_validate_logout",
  "gateway::tests::dispatcher_channels_status_rejects_unknown_params",
  "gateway::tests::dispatcher_channels_status_probe_false_sets_null_channel_last_probe_at",
  "gateway::tests::dispatcher_channels_logout_rejects_unknown_params",
  "gateway::tests::dispatcher_channels_logout_accepts_channel_alias",
  "gateway::tests::dispatcher_channels_status_reflects_runtime_event_snapshots",
  "gateway::tests::dispatcher_channels_status_tracks_payload_channel_alias_runtime",
  "gateway::tests::dispatcher_channels_logout_marks_runtime_offline",
  "gateway::tests::dispatcher_channels_logout_without_runtime_account_does_not_create_account",
  "gateway::tests::dispatcher_channels_status_ingests_channel_accounts_runtime_map",
  "gateway::tests::dispatcher_channels_status_honors_default_account_hints_from_runtime_payload",
  "gateway::tests::dispatcher_channels_status_ingests_nested_default_account_id_from_channels_map",
  "gateway::tests::dispatcher_channels_status_ingests_nested_snake_case_default_account_id_from_channels_map",
  "gateway::tests::dispatcher_channels_status_ingests_alias_channel_ids_in_runtime_maps",
  "gateway::tests::dispatcher_channels_status_ingests_snake_case_runtime_maps",
  "gateway::tests::dispatcher_channels_status_tracks_inbound_when_channel_is_only_in_payload",
  "gateway::tests::dispatcher_chat_send_updates_webchat_runtime_outbound_activity",
  "gateway::tests::dispatcher_channels_status_defaults_to_unconfigured_unlinked_without_runtime",
  "gateway::tests::dispatcher_channels_status_probe_false_sets_null_account_last_probe_at",
  "gateway::tests::dispatcher_channels_status_probe_true_sets_account_last_probe_at",
  "gateway::tests::dispatcher_channels_status_ingests_extended_account_metadata_fields",
  "gateway::tests::dispatcher_channels_status_ingests_runtime_probe_audit_and_application_payloads"
)

$toolchainArg = if ($Toolchain -and $Toolchain.Trim().Length -gt 0) {
  "+$($Toolchain.Trim())"
} else {
  ""
}

if (-not (Test-Path -LiteralPath $ArtifactDir)) {
  New-Item -ItemType Directory -Path $ArtifactDir -Force | Out-Null
}

$logFile = Join-Path $ArtifactDir "cp4-gate.log"
$resultsFile = Join-Path $ArtifactDir "cp4-fixture-results.tsv"
$summaryFile = Join-Path $ArtifactDir "cp4-gate-summary.md"
$metricsFile = Join-Path $ArtifactDir "cp4-metrics.json"

if (Test-Path -LiteralPath $logFile) {
  Remove-Item -LiteralPath $logFile -Force
}

$results = New-Object 'System.Collections.Generic.List[object]'
$totalDurationMs = 0

foreach ($testName in $tests) {
  $startMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  "[parity] running CP4 fixture: $testName" | Tee-Object -FilePath $logFile -Append | Out-Null

  if ($toolchainArg) {
    & $CargoCommand $toolchainArg test $testName -- --nocapture 2>&1 | Tee-Object -FilePath $logFile -Append | Out-Null
  } else {
    & $CargoCommand test $testName -- --nocapture 2>&1 | Tee-Object -FilePath $logFile -Append | Out-Null
  }

  $durationMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - $startMs
  $totalDurationMs += $durationMs

  if ($LASTEXITCODE -ne 0) {
    $results.Add([pscustomobject]@{
        test = $testName
        duration_ms = $durationMs
        status = "fail"
      }) | Out-Null
    throw "CP4 fixture failed: $testName (exit $LASTEXITCODE)"
  }

  $results.Add([pscustomobject]@{
      test = $testName
      duration_ms = $durationMs
      status = "pass"
    }) | Out-Null
}

$lines = @("test`tduration_ms`tstatus")
foreach ($result in $results) {
  $lines += "$($result.test)`t$($result.duration_ms)`t$($result.status)"
}
Set-Content -Path $resultsFile -Value $lines -Encoding utf8

$totalFixtures = $tests.Count
$avgDurationMs = if ($totalFixtures -gt 0) { [int]($totalDurationMs / $totalFixtures) } else { 0 }
$passed = ($results | Where-Object { $_.status -eq "pass" }).Count

$metrics = [ordered]@{
  gate = "cp4"
  passed = $passed
  totalFixtures = $totalFixtures
  totalDurationMs = $totalDurationMs
  avgFixtureDurationMs = $avgDurationMs
  resultsTsv = "cp4-fixture-results.tsv"
}
$metrics | ConvertTo-Json -Depth 5 | Set-Content -Path $metricsFile -Encoding utf8

$summary = @(
  "## CP4 Channel Runtime Wave-1 Foundation Gate",
  "",
  "- Fixtures passed: $passed/$totalFixtures",
  "- Total duration: $totalDurationMs ms",
  "- Avg fixture duration: $avgDurationMs ms",
  "- Artifact log: cp4-gate.log",
  "- Artifact metrics: cp4-metrics.json"
)
Set-Content -Path $summaryFile -Value $summary -Encoding utf8

"[parity] CP4 gate passed" | Tee-Object -FilePath $logFile -Append | Out-Null
