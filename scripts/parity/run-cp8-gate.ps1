param(
  [string]$CargoCommand = "cargo",
  [string]$Toolchain = "1.83.0-x86_64-pc-windows-gnu",
  [string]$ArtifactDir = "parity/generated/cp8"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$tests = @(
  "bridge::tests::replay_harness_with_real_defender",
  "bridge::tests::multi_session_soak_preserves_per_session_fifo_without_duplicates",
  "security::prompt_guard::tests::scores_prompt_injection_patterns",
  "security::command_guard::tests::blocks_known_destructive_patterns",
  "security::tests::tool_loop_detection_escalates_warning_then_critical",
  "security::policy_bundle::tests::loads_valid_signed_bundle_and_applies_policy_patch"
)

$toolchainArg = if ($Toolchain -and $Toolchain.Trim().Length -gt 0) {
  "+$($Toolchain.Trim())"
} else {
  ""
}

if (-not (Test-Path -LiteralPath $ArtifactDir)) {
  New-Item -ItemType Directory -Path $ArtifactDir -Force | Out-Null
}

$logFile = Join-Path $ArtifactDir "cp8-gate.log"
$resultsFile = Join-Path $ArtifactDir "cp8-fixture-results.tsv"
$summaryFile = Join-Path $ArtifactDir "cp8-gate-summary.md"
$metricsFile = Join-Path $ArtifactDir "cp8-metrics.json"

if (Test-Path -LiteralPath $logFile) {
  Remove-Item -LiteralPath $logFile -Force
}

$results = New-Object 'System.Collections.Generic.List[object]'
$totalDurationMs = 0
$reliabilityFixtures = 0
$securityFixtures = 0

foreach ($testName in $tests) {
  $startMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  "[parity] running CP8 fixture: $testName" | Tee-Object -FilePath $logFile -Append | Out-Null

  if ($toolchainArg) {
    & $CargoCommand $toolchainArg test $testName -- --nocapture 2>&1 | Tee-Object -FilePath $logFile -Append | Out-Null
  } else {
    & $CargoCommand test $testName -- --nocapture 2>&1 | Tee-Object -FilePath $logFile -Append | Out-Null
  }

  $durationMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - $startMs
  $totalDurationMs += $durationMs

  if ($testName.StartsWith("bridge::tests::")) {
    $reliabilityFixtures += 1
  } elseif ($testName.StartsWith("security::")) {
    $securityFixtures += 1
  }

  if ($LASTEXITCODE -ne 0) {
    $results.Add([pscustomobject]@{
        test = $testName
        duration_ms = $durationMs
        status = "fail"
      }) | Out-Null
    throw "CP8 fixture failed: $testName (exit $LASTEXITCODE)"
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
  gate = "cp8"
  passed = $passed
  totalFixtures = $totalFixtures
  totalDurationMs = $totalDurationMs
  avgFixtureDurationMs = $avgDurationMs
  reliabilityFixtureCount = $reliabilityFixtures
  securityFixtureCount = $securityFixtures
  resultsTsv = "cp8-fixture-results.tsv"
}
$metrics | ConvertTo-Json -Depth 5 | Set-Content -Path $metricsFile -Encoding utf8

$summary = @(
  "## CP8 Reliability + Security Hardening Starter Gate",
  "",
  "- Fixtures passed: $passed/$totalFixtures",
  "- Reliability fixtures: $reliabilityFixtures",
  "- Security fixtures: $securityFixtures",
  "- Total duration: $totalDurationMs ms",
  "- Avg fixture duration: $avgDurationMs ms",
  "- Artifact log: cp8-gate.log",
  "- Artifact metrics: cp8-metrics.json"
)
Set-Content -Path $summaryFile -Value $summary -Encoding utf8

"[parity] CP8 gate passed" | Tee-Object -FilePath $logFile -Append | Out-Null
