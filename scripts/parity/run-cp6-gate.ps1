param(
  [string]$CargoCommand = "cargo",
  [string]$Toolchain = "1.83.0-x86_64-pc-windows-gnu",
  [string]$ArtifactDir = "parity/generated/cp6"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$tests = @(
  "gateway::tests::dispatcher_models_list_returns_catalog_and_rejects_unknown_params",
  "gateway::tests::dispatcher_patch_model_normalizes_provider_aliases_and_failover_provider_rules",
  "gateway::tests::model_provider_failover_chain_normalizes_aliases",
  "security::tool_policy::tests::provider_specific_rule_is_applied_after_global_policy",
  "security::tool_policy::tests::provider_model_specific_rule_beats_provider_fallback"
)

$toolchainArg = if ($Toolchain -and $Toolchain.Trim().Length -gt 0) {
  "+$($Toolchain.Trim())"
} else {
  ""
}

if (-not (Test-Path -LiteralPath $ArtifactDir)) {
  New-Item -ItemType Directory -Path $ArtifactDir -Force | Out-Null
}

$logFile = Join-Path $ArtifactDir "cp6-gate.log"
$resultsFile = Join-Path $ArtifactDir "cp6-fixture-results.tsv"
$summaryFile = Join-Path $ArtifactDir "cp6-gate-summary.md"
$metricsFile = Join-Path $ArtifactDir "cp6-metrics.json"

if (Test-Path -LiteralPath $logFile) {
  Remove-Item -LiteralPath $logFile -Force
}

$results = New-Object 'System.Collections.Generic.List[object]'
$totalDurationMs = 0

foreach ($testName in $tests) {
  $startMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  "[parity] running CP6 fixture: $testName" | Tee-Object -FilePath $logFile -Append | Out-Null

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
    throw "CP6 fixture failed: $testName (exit $LASTEXITCODE)"
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
  gate = "cp6"
  passed = $passed
  totalFixtures = $totalFixtures
  totalDurationMs = $totalDurationMs
  avgFixtureDurationMs = $avgDurationMs
  resultsTsv = "cp6-fixture-results.tsv"
}
$metrics | ConvertTo-Json -Depth 5 | Set-Content -Path $metricsFile -Encoding utf8

$summary = @(
  "## CP6 Model Provider/Auth/Failover Foundation Gate",
  "",
  "- Fixtures passed: $passed/$totalFixtures",
  "- Total duration: $totalDurationMs ms",
  "- Avg fixture duration: $avgDurationMs ms",
  "- Artifact log: cp6-gate.log",
  "- Artifact metrics: cp6-metrics.json"
)
Set-Content -Path $summaryFile -Value $summary -Encoding utf8

"[parity] CP6 gate passed" | Tee-Object -FilePath $logFile -Append | Out-Null
