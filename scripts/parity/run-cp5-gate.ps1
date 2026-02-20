param(
  [string]$CargoCommand = "cargo",
  [string]$Toolchain = "1.83.0-x86_64-pc-windows-gnu",
  [string]$ArtifactDir = "parity/generated/cp5"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
  $PSNativeCommandUseErrorActionPreference = $false
}

$tests = @(
  "gateway::tests::dispatcher_browser_request_validates_and_reports_unavailable_contract",
  "gateway::tests::dispatcher_browser_request_routes_through_node_proxy_runtime",
  "gateway::tests::dispatcher_browser_request_enforces_browser_proxy_command_allowlist",
  "gateway::tests::dispatcher_browser_open_routes_through_browser_proxy_runtime",
  "gateway::tests::dispatcher_canvas_present_routes_through_node_runtime",
  "gateway::tests::dispatcher_canvas_present_rejects_disallowed_command",
  "gateway::tests::dispatcher_device_pair_and_token_methods_follow_parity_contract",
  "gateway::tests::dispatcher_node_pairing_methods_follow_parity_contract",
  "gateway::tests::dispatcher_node_invoke_and_event_methods_follow_parity_contract",
  "gateway::tests::dispatcher_node_invoke_supports_camera_screen_location_and_system_commands_when_declared",
  "gateway::tests::dispatcher_local_node_host_runtime_command_override_map_routes_by_command"
)

$toolchainArg = if ($Toolchain -and $Toolchain.Trim().Length -gt 0) {
  "+$($Toolchain.Trim())"
} else {
  ""
}

if (-not (Test-Path -LiteralPath $ArtifactDir)) {
  New-Item -ItemType Directory -Path $ArtifactDir -Force | Out-Null
}

$logFile = Join-Path $ArtifactDir "cp5-gate.log"
$resultsFile = Join-Path $ArtifactDir "cp5-fixture-results.tsv"
$summaryFile = Join-Path $ArtifactDir "cp5-gate-summary.md"
$metricsFile = Join-Path $ArtifactDir "cp5-metrics.json"

if (Test-Path -LiteralPath $logFile) {
  Remove-Item -LiteralPath $logFile -Force
}

$results = New-Object 'System.Collections.Generic.List[object]'
$totalDurationMs = 0

foreach ($testName in $tests) {
  $startMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  "[parity] running CP5 fixture: $testName" | Tee-Object -FilePath $logFile -Append | Out-Null

  $previousErrorActionPreference = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  try {
    if ($toolchainArg) {
      & $CargoCommand $toolchainArg test $testName -- --nocapture *>&1 | Tee-Object -FilePath $logFile -Append | Out-Null
    } else {
      & $CargoCommand test $testName -- --nocapture *>&1 | Tee-Object -FilePath $logFile -Append | Out-Null
    }
  } finally {
    $ErrorActionPreference = $previousErrorActionPreference
  }

  $durationMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - $startMs
  $totalDurationMs += $durationMs

  if ($LASTEXITCODE -ne 0) {
    $results.Add([pscustomobject]@{
        test = $testName
        duration_ms = $durationMs
        status = "fail"
      }) | Out-Null
    throw "CP5 fixture failed: $testName (exit $LASTEXITCODE)"
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
  gate = "cp5"
  passed = $passed
  totalFixtures = $totalFixtures
  totalDurationMs = $totalDurationMs
  avgFixtureDurationMs = $avgDurationMs
  resultsTsv = "cp5-fixture-results.tsv"
}
$metrics | ConvertTo-Json -Depth 5 | Set-Content -Path $metricsFile -Encoding utf8

$summary = @(
  "## CP5 Nodes + Browser + Canvas + Device Gate",
  "",
  "- Fixtures passed: $passed/$totalFixtures",
  "- Total duration: $totalDurationMs ms",
  "- Avg fixture duration: $avgDurationMs ms",
  "- Artifact log: cp5-gate.log",
  "- Artifact metrics: cp5-metrics.json"
)
Set-Content -Path $summaryFile -Value $summary -Encoding utf8

"[parity] CP5 gate passed" | Tee-Object -FilePath $logFile -Append | Out-Null

