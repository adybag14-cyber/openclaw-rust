param(
  [string]$CargoCommand = "cargo",
  [string]$Toolchain = "1.83.0-x86_64-pc-windows-gnu",
  [string]$ArtifactDir = "parity/generated/cp7"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
  $PSNativeCommandUseErrorActionPreference = $false
}

$tests = @(
  "tests::cli_parses_doctor_command_and_flags",
  "tests::cli_parses_security_audit_command_and_flags",
  "tests::cli_parses_gateway_status_command_and_json_flag",
  "tests::cli_parses_message_send_command",
  "tests::doctor_report_marks_config_load_failure_as_blocking",
  "tests::doctor_report_warns_when_docker_is_unavailable",
  "tests::cli_dispatch_rpc_status_returns_runtime_payload",
  "tests::cli_dispatch_rpc_send_returns_message_identifiers",
  "tests::cli_dispatch_rpc_sessions_list_returns_count_field",
  "gateway::tests::dispatcher_update_and_web_login_methods_report_expected_payloads"
)

$toolchainArg = if ($Toolchain -and $Toolchain.Trim().Length -gt 0) {
  "+$($Toolchain.Trim())"
} else {
  ""
}

if (-not (Test-Path -LiteralPath $ArtifactDir)) {
  New-Item -ItemType Directory -Path $ArtifactDir -Force | Out-Null
}

$logFile = Join-Path $ArtifactDir "cp7-gate.log"
$resultsFile = Join-Path $ArtifactDir "cp7-fixture-results.tsv"
$summaryFile = Join-Path $ArtifactDir "cp7-gate-summary.md"
$metricsFile = Join-Path $ArtifactDir "cp7-metrics.json"

if (Test-Path -LiteralPath $logFile) {
  Remove-Item -LiteralPath $logFile -Force
}

$results = New-Object 'System.Collections.Generic.List[object]'
$totalDurationMs = 0

foreach ($testName in $tests) {
  $startMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  "[parity] running CP7 fixture: $testName" | Tee-Object -FilePath $logFile -Append | Out-Null

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
    throw "CP7 fixture failed: $testName (exit $LASTEXITCODE)"
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
  gate = "cp7"
  passed = $passed
  totalFixtures = $totalFixtures
  totalDurationMs = $totalDurationMs
  avgFixtureDurationMs = $avgDurationMs
  resultsTsv = "cp7-fixture-results.tsv"
}
$metrics | ConvertTo-Json -Depth 5 | Set-Content -Path $metricsFile -Encoding utf8

$summary = @(
  "## CP7 CLI + Control UI Parity Gate",
  "",
  "- Fixtures passed: $passed/$totalFixtures",
  "- Total duration: $totalDurationMs ms",
  "- Avg fixture duration: $avgDurationMs ms",
  "- Artifact log: cp7-gate.log",
  "- Artifact metrics: cp7-metrics.json"
)
Set-Content -Path $summaryFile -Value $summary -Encoding utf8

"[parity] CP7 gate passed" | Tee-Object -FilePath $logFile -Append | Out-Null

