param(
  [string]$ArtifactDir = "parity/generated/cp9"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
  $PSNativeCommandUseErrorActionPreference = $false
}

if (-not (Test-Path -LiteralPath $ArtifactDir)) {
  New-Item -ItemType Directory -Path $ArtifactDir -Force | Out-Null
}

$logFile = Join-Path $ArtifactDir "cp9-gate.log"
$resultsFile = Join-Path $ArtifactDir "cp9-check-results.tsv"
$summaryFile = Join-Path $ArtifactDir "cp9-gate-summary.md"
$metricsFile = Join-Path $ArtifactDir "cp9-metrics.json"

if (Test-Path -LiteralPath $logFile) {
  Remove-Item -LiteralPath $logFile -Force
}
Set-Content -Path $resultsFile -Value "check`tduration_ms`tstatus" -Encoding utf8

$checksRun = 0
$passed = 0
$totalDurationMs = 0
$overallStatus = "pass"
$dockerServerVersion = "unavailable"
$dockerComposeVersion = "unavailable"

$results = New-Object 'System.Collections.Generic.List[object]'

function Invoke-Cp9Check {
  param(
    [string]$Name,
    [string[]]$Command
  )

  $script:checksRun += 1
  $startMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  "[parity] running CP9 check: $Name" | Tee-Object -FilePath $script:logFile -Append | Out-Null

  $previousErrorActionPreference = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  try {
    & $Command[0] $Command[1..($Command.Length - 1)] *>&1 | Tee-Object -FilePath $script:logFile -Append | Out-Null
  } finally {
    $ErrorActionPreference = $previousErrorActionPreference
  }

  $durationMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - $startMs
  $script:totalDurationMs += $durationMs

  if ($LASTEXITCODE -ne 0) {
    $script:results.Add([pscustomobject]@{
        check = $Name
        duration_ms = $durationMs
        status = "fail"
      }) | Out-Null
    $script:overallStatus = "fail"
    return $false
  }

  $script:results.Add([pscustomobject]@{
      check = $Name
      duration_ms = $durationMs
      status = "pass"
    }) | Out-Null
  $script:passed += 1
  return $true
}

if (Invoke-Cp9Check -Name "docker-daemon" -Command @("docker", "info")) {
  try {
    $dockerServerVersion = (docker version --format "{{.Server.Version}}" 2>$null).Trim()
    if (-not $dockerServerVersion) {
      $dockerServerVersion = "unknown"
    }
  } catch {
    $dockerServerVersion = "unknown"
  }
  try {
    $dockerComposeVersion = (docker compose version --short 2>$null).Trim()
    if (-not $dockerComposeVersion) {
      $dockerComposeVersion = "unknown"
    }
  } catch {
    $dockerComposeVersion = "unknown"
  }

  Invoke-Cp9Check -Name "docker-smoke" -Command @(
    "powershell",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    "scripts/run-docker-parity-smoke.ps1"
  ) | Out-Null
  if ($overallStatus -eq "pass") {
    Invoke-Cp9Check -Name "docker-compose-parity" -Command @(
      "powershell",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      "scripts/run-docker-compose-parity.ps1"
    ) | Out-Null
  }
  if ($overallStatus -eq "pass") {
    Invoke-Cp9Check -Name "docker-compose-chaos-restart" -Command @(
      "powershell",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      "scripts/run-docker-compose-parity-chaos.ps1"
    ) | Out-Null
  }
}

$resultLines = @("check`tduration_ms`tstatus")
foreach ($result in $results) {
  $resultLines += "$($result.check)`t$($result.duration_ms)`t$($result.status)"
}
Set-Content -Path $resultsFile -Value $resultLines -Encoding utf8

$failed = $checksRun - $passed
$avgDurationMs = if ($checksRun -gt 0) { [int]($totalDurationMs / $checksRun) } else { 0 }

$metrics = [ordered]@{
  gate = "cp9"
  status = $overallStatus
  checksRun = $checksRun
  checksPassed = $passed
  checksFailed = $failed
  totalDurationMs = $totalDurationMs
  avgCheckDurationMs = $avgDurationMs
  dockerServerVersion = $dockerServerVersion
  dockerComposeVersion = $dockerComposeVersion
  resultsTsv = "cp9-check-results.tsv"
}
$metrics | ConvertTo-Json -Depth 4 | Set-Content -Path $metricsFile -Encoding utf8

$summary = @(
  "## CP9 Docker End-to-End Parity Gate",
  "",
  "- Checks passed: $passed/$checksRun",
  "- Checks failed: $failed",
  "- Total duration: $totalDurationMs ms",
  "- Avg check duration: $avgDurationMs ms",
  "- Docker server version: $dockerServerVersion",
  "- Docker compose version: $dockerComposeVersion",
  "- Artifact log: cp9-gate.log",
  "- Artifact metrics: cp9-metrics.json",
  "- Artifact results: cp9-check-results.tsv"
)
Set-Content -Path $summaryFile -Value $summary -Encoding utf8

if ($overallStatus -ne "pass") {
  "[parity] CP9 gate failed" | Tee-Object -FilePath $logFile -Append | Out-Null
  exit 1
}

"[parity] CP9 gate passed" | Tee-Object -FilePath $logFile -Append | Out-Null
