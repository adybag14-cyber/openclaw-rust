param(
  [string]$CargoCommand = "cargo",
  [string]$Toolchain = "1.83.0-x86_64-pc-windows-gnu",
  [string]$ArtifactDir = "parity/generated/cp2"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
  $PSNativeCommandUseErrorActionPreference = $false
}

$defaultTests = @(
  "bridge::tests::steer_mode_keeps_latest_pending_at_bridge_level",
  "bridge::tests::followup_queue_pressure_preserves_order_without_duplicates",
  "bridge::tests::session_routing_corpus_matches_expected_delivery_order",
  "bridge::tests::multi_session_soak_preserves_per_session_fifo_without_duplicates",
  "bridge::tests::reply_back_payload_preserves_group_and_direct_delivery_context",
  "gateway::tests::dispatcher_list_supports_label_spawn_filters_and_message_hints",
  "gateway::tests::dispatcher_list_route_selectors_disambiguate_shared_peer_by_account_and_channel",
  "gateway::tests::dispatcher_resolve_supports_label_agent_and_spawn_filters",
  "gateway::tests::dispatcher_resolve_route_selectors_disambiguate_shared_peer_by_account_and_channel",
  "gateway::tests::dispatcher_resolve_prefers_explicit_session_key_over_route_selectors",
  "gateway::tests::dispatcher_resolve_prefers_session_id_over_label_and_route_selectors",
  "gateway::tests::dispatcher_resolve_supports_label_plus_route_selectors",
  "gateway::tests::dispatcher_resolve_accepts_partial_route_selectors_without_account_id",
  "gateway::tests::dispatcher_resolve_partial_route_collision_prefers_most_recent_update",
  "gateway::tests::dispatcher_resolve_partial_route_collision_uses_key_tiebreak_when_timestamps_match"
)

$sqliteTests = @(
  "state::tests::sqlite_state_survives_restart_and_continues_counters",
  "state::tests::sqlite_state_recovers_multiple_sessions_after_restart"
)

$toolchainArg = if ($Toolchain -and $Toolchain.Trim().Length -gt 0) {
  "+$($Toolchain.Trim())"
} else {
  ""
}

if (-not (Test-Path -LiteralPath $ArtifactDir)) {
  New-Item -ItemType Directory -Path $ArtifactDir -Force | Out-Null
}

$logFile = Join-Path $ArtifactDir "cp2-gate.log"
$resultsFile = Join-Path $ArtifactDir "cp2-fixture-results.tsv"
$summaryFile = Join-Path $ArtifactDir "cp2-gate-summary.md"
$metricsFile = Join-Path $ArtifactDir "cp2-metrics.json"

if (Test-Path -LiteralPath $logFile) {
  Remove-Item -LiteralPath $logFile -Force
}

$results = New-Object 'System.Collections.Generic.List[object]'
$totalDurationMs = 0
$soakDurationMs = 0
$soakCount = 0

function Invoke-Cp2Fixture {
  param(
    [string]$Suite,
    [string]$TestName,
    [switch]$SqliteFeature
  )

  $startMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  "[parity] running CP2 $Suite fixture: $TestName" | Tee-Object -FilePath $logFile -Append | Out-Null

  $previousErrorActionPreference = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  try {
    if ($toolchainArg) {
      if ($SqliteFeature) {
        & $CargoCommand $toolchainArg test --features sqlite-state $TestName -- --nocapture *>&1 | Tee-Object -FilePath $logFile -Append | Out-Null
      } else {
        & $CargoCommand $toolchainArg test $TestName -- --nocapture *>&1 | Tee-Object -FilePath $logFile -Append | Out-Null
      }
    } else {
      if ($SqliteFeature) {
        & $CargoCommand test --features sqlite-state $TestName -- --nocapture *>&1 | Tee-Object -FilePath $logFile -Append | Out-Null
      } else {
        & $CargoCommand test $TestName -- --nocapture *>&1 | Tee-Object -FilePath $logFile -Append | Out-Null
      }
    }
  } finally {
    $ErrorActionPreference = $previousErrorActionPreference
  }

  $durationMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - $startMs
  $script:totalDurationMs += $durationMs

  if ($LASTEXITCODE -ne 0) {
    $script:results.Add([pscustomobject]@{
        suite = $Suite
        test = $TestName
        duration_ms = $durationMs
        status = "fail"
      }) | Out-Null
    throw "CP2 fixture failed: $Suite/$TestName (exit $LASTEXITCODE)"
  }

  if ($TestName -like "*soak*" -or $TestName -like "*queue_pressure*" -or $TestName -like "*delivery_order*") {
    $script:soakCount += 1
    $script:soakDurationMs += $durationMs
  }

  $script:results.Add([pscustomobject]@{
      suite = $Suite
      test = $TestName
      duration_ms = $durationMs
      status = "pass"
    }) | Out-Null
}

foreach ($testName in $defaultTests) {
  Invoke-Cp2Fixture -Suite "default" -TestName $testName
}

foreach ($testName in $sqliteTests) {
  Invoke-Cp2Fixture -Suite "sqlite-feature" -TestName $testName -SqliteFeature
}

$lines = @("suite`ttest`tduration_ms`tstatus")
foreach ($result in $results) {
  $lines += "$($result.suite)`t$($result.test)`t$($result.duration_ms)`t$($result.status)"
}
Set-Content -Path $resultsFile -Value $lines -Encoding utf8

Copy-Item -Path "tests/parity/session-routing-corpus.json" -Destination (Join-Path $ArtifactDir "session-routing-corpus.json") -Force
Copy-Item -Path "tests/parity/gateway-payload-corpus.json" -Destination (Join-Path $ArtifactDir "gateway-payload-corpus.json") -Force

$totalFixtures = $results.Count
$avgDurationMs = if ($totalFixtures -gt 0) { [int]($totalDurationMs / $totalFixtures) } else { 0 }

$metrics = [ordered]@{
  gate = "cp2"
  defaultPassed = ($results | Where-Object { $_.suite -eq "default" -and $_.status -eq "pass" }).Count
  sqliteFeaturePassed = ($results | Where-Object { $_.suite -eq "sqlite-feature" -and $_.status -eq "pass" }).Count
  totalFixtures = $totalFixtures
  totalDurationMs = $totalDurationMs
  avgFixtureDurationMs = $avgDurationMs
  soakFixtureCount = $soakCount
  soakFixtureDurationMs = $soakDurationMs
  resultsTsv = "cp2-fixture-results.tsv"
}
$metrics | ConvertTo-Json -Depth 5 | Set-Content -Path $metricsFile -Encoding utf8

$summary = @(
  "## CP2 Session/Routing Gate",
  "",
  "- Default fixtures passed: $($metrics.defaultPassed)",
  "- SQLite feature fixtures passed: $($metrics.sqliteFeaturePassed)",
  "- Total fixtures: $($metrics.totalFixtures)",
  "- Total duration: $($metrics.totalDurationMs) ms",
  "- Avg fixture duration: $($metrics.avgFixtureDurationMs) ms",
  "- Soak/order fixtures: $($metrics.soakFixtureCount)",
  "- Soak/order fixture duration: $($metrics.soakFixtureDurationMs) ms",
  "- Artifact log: cp2-gate.log",
  "- Artifact metrics: cp2-metrics.json"
)
Set-Content -Path $summaryFile -Value $summary -Encoding utf8

"[parity] CP2 gate passed" | Tee-Object -FilePath $logFile -Append | Out-Null

