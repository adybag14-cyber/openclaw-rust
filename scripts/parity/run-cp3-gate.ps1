param(
  [string]$CargoCommand = "cargo",
  [string]$Toolchain = "1.83.0-x86_64-pc-windows-gnu",
  [string]$ArtifactDir = "parity/generated/cp3"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$tests = @(
  "security::tool_policy::tests::profile_coding_expands_group_runtime_and_fs",
  "security::tool_policy::tests::deny_takes_precedence_over_allow",
  "security::tool_policy::tests::provider_specific_rule_is_applied_after_global_policy",
  "security::tool_policy::tests::provider_model_specific_rule_beats_provider_fallback",
  "security::tool_policy::tests::allowlisted_exec_implies_apply_patch",
  "security::tool_loop::tests::emits_warning_and_critical_on_repeated_identical_tool_calls",
  "security::tests::tool_runtime_policy_profile_blocks_non_profile_tools",
  "security::tests::tool_loop_detection_escalates_warning_then_critical",
  "tool_runtime::tests::tool_runtime_corpus_matches_expected_outcomes",
  "tool_runtime::tests::tool_runtime_policy_and_loop_guard_enforced_on_tool_host",
  "tool_runtime::tests::tool_runtime_background_exec_process_poll_roundtrip"
)

$toolchainArg = if ($Toolchain -and $Toolchain.Trim().Length -gt 0) {
  "+$($Toolchain.Trim())"
} else {
  ""
}

if (-not (Test-Path -LiteralPath $ArtifactDir)) {
  New-Item -ItemType Directory -Path $ArtifactDir -Force | Out-Null
}

$logFile = Join-Path $ArtifactDir "cp3-gate.log"
$resultsFile = Join-Path $ArtifactDir "cp3-fixture-results.tsv"
$summaryFile = Join-Path $ArtifactDir "cp3-gate-summary.md"
$metricsFile = Join-Path $ArtifactDir "cp3-metrics.json"
$corpusArtifact = Join-Path $ArtifactDir "tool-runtime-corpus.json"

if (Test-Path -LiteralPath $logFile) {
  Remove-Item -LiteralPath $logFile -Force
}

$results = New-Object 'System.Collections.Generic.List[object]'
$totalDurationMs = 0

foreach ($testName in $tests) {
  $startMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  "[parity] running CP3 fixture: $testName" | Tee-Object -FilePath $logFile -Append | Out-Null

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
    throw "CP3 fixture failed: $testName (exit $LASTEXITCODE)"
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
Copy-Item -Path "tests/parity/tool-runtime-corpus.json" -Destination $corpusArtifact -Force

$totalFixtures = $tests.Count
$avgDurationMs = if ($totalFixtures -gt 0) { [int]($totalDurationMs / $totalFixtures) } else { 0 }
$passed = ($results | Where-Object { $_.status -eq "pass" }).Count

$metrics = [ordered]@{
  gate = "cp3"
  passed = $passed
  totalFixtures = $totalFixtures
  totalDurationMs = $totalDurationMs
  avgFixtureDurationMs = $avgDurationMs
  resultsTsv = "cp3-fixture-results.tsv"
}
$metrics | ConvertTo-Json -Depth 5 | Set-Content -Path $metricsFile -Encoding utf8

$summary = @(
  "## CP3 Tool Runtime Parity Gate",
  "",
  "- Fixtures passed: $passed/$totalFixtures",
  "- Total duration: $totalDurationMs ms",
  "- Avg fixture duration: $avgDurationMs ms",
  "- Artifact log: cp3-gate.log",
  "- Artifact metrics: cp3-metrics.json",
  "- Fixture corpus: tool-runtime-corpus.json"
)
Set-Content -Path $summaryFile -Value $summary -Encoding utf8

"[parity] CP3 gate passed" | Tee-Object -FilePath $logFile -Append | Out-Null
