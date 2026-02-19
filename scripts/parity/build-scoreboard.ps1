param(
  [string]$AuditPath = "OPENCLAW_FEATURE_AUDIT.md",
  [string]$MethodDiffPath = "parity/generated/method-surface-diff.json",
  [string]$ManifestPath = "parity/manifest/PARITY_MANIFEST.v1.json",
  [string]$BaselinePath = "parity/manifest/scoreboard-baseline.json",
  [string]$OutJson = "parity/generated/parity-scoreboard.json",
  [string]$OutMarkdown = "parity/generated/parity-scoreboard.md",
  [switch]$WriteBaseline,
  [switch]$IncludeGeneratedAt
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Ensure-ParentDir {
  param([string]$Path)
  $parent = Split-Path -Parent $Path
  if ($parent -and -not (Test-Path -LiteralPath $parent)) {
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
  }
}

function Get-OptionalProperty {
  param(
    [object]$Object,
    [string]$Name
  )
  if ($null -eq $Object) {
    return $null
  }
  if ($Object.PSObject.Properties.Name -contains $Name) {
    return $Object.$Name
  }
  return $null
}

function Parse-AuditStatus {
  param([string]$Path)

  if (-not (Test-Path -LiteralPath $Path)) {
    throw "Audit file not found: $Path"
  }

  $featureStatus = [ordered]@{}
  $counts = [ordered]@{
    total = 0
    implemented = 0
    partial = 0
    deferred = 0
    notStarted = 0
    unknown = 0
  }

  $pattern = '^- `([^`]+)`:.*Rust status is `([^`]+)`'
  foreach ($line in (Get-Content -LiteralPath $Path)) {
    if ($line -notmatch $pattern) {
      continue
    }
    $feature = $matches[1].Trim()
    $status = $matches[2].Trim()
    $featureStatus[$feature] = $status
    $counts.total += 1
    switch ($status.ToLowerInvariant()) {
      "implemented" { $counts.implemented += 1; break }
      "partial" { $counts.partial += 1; break }
      "deferred" { $counts.deferred += 1; break }
      "not started" { $counts.notStarted += 1; break }
      default { $counts.unknown += 1; break }
    }
  }

  return [ordered]@{
    counts = $counts
    byFeature = $featureStatus
  }
}

function Normalize-SurfaceMetrics {
  param([object]$Surface)
  if ($null -eq $Surface) {
    return $null
  }
  $missing = @(Get-OptionalProperty -Object $Surface -Name "missingInRust")
  $rustOnly = @(Get-OptionalProperty -Object $Surface -Name "rustOnly")
  return [ordered]@{
    upstreamCount = [int](Get-OptionalProperty -Object $Surface -Name "upstreamCount")
    rustCount = [int](Get-OptionalProperty -Object $Surface -Name "rustCount")
    intersectionCount = [int](Get-OptionalProperty -Object $Surface -Name "intersectionCount")
    coveragePercent = [double](Get-OptionalProperty -Object $Surface -Name "coveragePercent")
    missingInRustCount = $missing.Count
    rustOnlyCount = $rustOnly.Count
  }
}

if (-not (Test-Path -LiteralPath $MethodDiffPath)) {
  throw "Method diff JSON not found: $MethodDiffPath"
}

$manifest = $null
if (Test-Path -LiteralPath $ManifestPath) {
  $manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
}

$methodDiff = Get-Content -LiteralPath $MethodDiffPath -Raw | ConvertFrom-Json
$surfaces = Get-OptionalProperty -Object $methodDiff -Name "surfaces"
$baseSurface = Normalize-SurfaceMetrics -Surface (Get-OptionalProperty -Object $surfaces -Name "base")
$handlerSurface = Normalize-SurfaceMetrics -Surface (Get-OptionalProperty -Object $surfaces -Name "handlers")
$audit = Parse-AuditStatus -Path $AuditPath

$current = [ordered]@{
  manifestVersion = (Get-OptionalProperty -Object $manifest -Name "manifestVersion")
  methodSurface = [ordered]@{
    rustCount = [int](Get-OptionalProperty -Object $methodDiff -Name "rustCount")
    base = $baseSurface
    handlers = $handlerSurface
  }
  audit = $audit
}
if ($IncludeGeneratedAt) {
  $current["generatedAtUtc"] = [DateTime]::UtcNow.ToString("o")
}

$baseline = $null
if (Test-Path -LiteralPath $BaselinePath) {
  $baseline = Get-Content -LiteralPath $BaselinePath -Raw | ConvertFrom-Json
}

function Delta-Int {
  param([object]$CurrentValue, [object]$BaselineValue)
  if ($null -eq $BaselineValue) {
    return $null
  }
  return ([int]$CurrentValue) - ([int]$BaselineValue)
}

function Delta-Double {
  param([object]$CurrentValue, [object]$BaselineValue)
  if ($null -eq $BaselineValue) {
    return $null
  }
  return [Math]::Round(([double]$CurrentValue) - ([double]$BaselineValue), 2)
}

$delta = [ordered]@{
  audit = $null
  methodSurface = $null
  changedSubsystems = @()
}

if ($baseline) {
  $delta["audit"] = [ordered]@{
    implemented = Delta-Int -CurrentValue $current.audit.counts.implemented -BaselineValue $baseline.audit.counts.implemented
    partial = Delta-Int -CurrentValue $current.audit.counts.partial -BaselineValue $baseline.audit.counts.partial
    deferred = Delta-Int -CurrentValue $current.audit.counts.deferred -BaselineValue $baseline.audit.counts.deferred
    notStarted = Delta-Int -CurrentValue $current.audit.counts.notStarted -BaselineValue $baseline.audit.counts.notStarted
  }

  $delta["methodSurface"] = [ordered]@{
    rustCount = Delta-Int -CurrentValue $current.methodSurface.rustCount -BaselineValue $baseline.methodSurface.rustCount
    baseCoveragePercent = Delta-Double -CurrentValue $current.methodSurface.base.coveragePercent -BaselineValue $baseline.methodSurface.base.coveragePercent
    handlersCoveragePercent = Delta-Double -CurrentValue $current.methodSurface.handlers.coveragePercent -BaselineValue $baseline.methodSurface.handlers.coveragePercent
    baseMissingInRustCount = Delta-Int -CurrentValue $current.methodSurface.base.missingInRustCount -BaselineValue $baseline.methodSurface.base.missingInRustCount
    handlersMissingInRustCount = Delta-Int -CurrentValue $current.methodSurface.handlers.missingInRustCount -BaselineValue $baseline.methodSurface.handlers.missingInRustCount
  }

  $baselineMap = [ordered]@{}
  foreach ($prop in $baseline.audit.byFeature.PSObject.Properties) {
    $baselineMap[$prop.Name] = [string]$prop.Value
  }
  $currentMap = [ordered]@{}
  foreach ($prop in $current.audit.byFeature.GetEnumerator()) {
    $currentMap[$prop.Key] = [string]$prop.Value
  }

  $changed = New-Object 'System.Collections.Generic.List[object]'
  $allKeys = New-Object 'System.Collections.Generic.HashSet[string]'
  foreach ($k in $baselineMap.Keys) { [void]$allKeys.Add($k) }
  foreach ($k in $currentMap.Keys) { [void]$allKeys.Add($k) }
  foreach ($key in @($allKeys) | Sort-Object) {
    $before = if ($baselineMap.Contains($key)) { $baselineMap[$key] } else { $null }
    $after = if ($currentMap.Contains($key)) { $currentMap[$key] } else { $null }
    if ($before -ne $after) {
      $changed.Add([ordered]@{
        subsystem = $key
        baseline = $before
        current = $after
      }) | Out-Null
    }
  }
  $delta["changedSubsystems"] = @($changed.ToArray())
}

$scoreboard = [ordered]@{
  current = $current
  baselinePresent = [bool]$baseline
  delta = $delta
}

Ensure-ParentDir -Path $OutJson
Ensure-ParentDir -Path $OutMarkdown
$scoreboard | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutJson

function Format-Delta {
  param([object]$Value)
  if ($null -eq $Value) {
    return "n/a"
  }
  if ([double]$Value -gt 0) {
    return "+$Value"
  }
  return "$Value"
}

$summaryLines = New-Object 'System.Collections.Generic.List[string]'
$summaryLines.Add("# Parity Scoreboard") | Out-Null
$summaryLines.Add("") | Out-Null
$generatedAt = if ($current.Contains("generatedAtUtc")) { $current.generatedAtUtc } else { "deterministic" }
$summaryLines.Add("Generated (UTC): $generatedAt") | Out-Null
$summaryLines.Add("") | Out-Null
$summaryLines.Add("## Current Snapshot") | Out-Null
$summaryLines.Add("") | Out-Null
$summaryLines.Add("| Metric | Value |") | Out-Null
$summaryLines.Add("| --- | ---: |") | Out-Null
$summaryLines.Add("| Audit Implemented | $($current.audit.counts.implemented) |") | Out-Null
$summaryLines.Add("| Audit Partial | $($current.audit.counts.partial) |") | Out-Null
$summaryLines.Add("| Audit Deferred | $($current.audit.counts.deferred) |") | Out-Null
$summaryLines.Add("| Audit Not Started | $($current.audit.counts.notStarted) |") | Out-Null
$summaryLines.Add("| Rust RPC Methods | $($current.methodSurface.rustCount) |") | Out-Null
$summaryLines.Add("| Base Coverage (%) | $($current.methodSurface.base.coveragePercent) |") | Out-Null
$summaryLines.Add("| Base Missing In Rust | $($current.methodSurface.base.missingInRustCount) |") | Out-Null
$summaryLines.Add("| Handlers Coverage (%) | $($current.methodSurface.handlers.coveragePercent) |") | Out-Null
$summaryLines.Add("| Handlers Missing In Rust | $($current.methodSurface.handlers.missingInRustCount) |") | Out-Null
$summaryLines.Add("") | Out-Null

if ($baseline) {
  $summaryLines.Add("## Delta vs Baseline") | Out-Null
  $summaryLines.Add("") | Out-Null
  $summaryLines.Add("| Metric | Delta |") | Out-Null
  $summaryLines.Add("| --- | ---: |") | Out-Null
  $summaryLines.Add("| Implemented | $(Format-Delta -Value $delta.audit.implemented) |") | Out-Null
  $summaryLines.Add("| Partial | $(Format-Delta -Value $delta.audit.partial) |") | Out-Null
  $summaryLines.Add("| Deferred | $(Format-Delta -Value $delta.audit.deferred) |") | Out-Null
  $summaryLines.Add("| Not Started | $(Format-Delta -Value $delta.audit.notStarted) |") | Out-Null
  $summaryLines.Add("| Rust RPC Methods | $(Format-Delta -Value $delta.methodSurface.rustCount) |") | Out-Null
  $summaryLines.Add("| Base Coverage (%) | $(Format-Delta -Value $delta.methodSurface.baseCoveragePercent) |") | Out-Null
  $summaryLines.Add("| Base Missing In Rust | $(Format-Delta -Value $delta.methodSurface.baseMissingInRustCount) |") | Out-Null
  $summaryLines.Add("| Handlers Coverage (%) | $(Format-Delta -Value $delta.methodSurface.handlersCoveragePercent) |") | Out-Null
  $summaryLines.Add("| Handlers Missing In Rust | $(Format-Delta -Value $delta.methodSurface.handlersMissingInRustCount) |") | Out-Null
  $summaryLines.Add("") | Out-Null
  $summaryLines.Add("## Subsystem Status Deltas") | Out-Null
  $summaryLines.Add("") | Out-Null
  if (@($delta.changedSubsystems).Count -eq 0) {
    $summaryLines.Add("- _No subsystem status changes vs baseline._") | Out-Null
  } else {
    foreach ($item in @($delta.changedSubsystems)) {
      $summaryLines.Add("- " + $item.subsystem + ": " + $item.baseline + " -> " + $item.current) | Out-Null
    }
  }
  $summaryLines.Add("") | Out-Null
} else {
  $summaryLines.Add("## Delta vs Baseline") | Out-Null
  $summaryLines.Add("") | Out-Null
  $summaryLines.Add("- _Baseline file not found; no delta computed._") | Out-Null
  $summaryLines.Add("") | Out-Null
}

$summaryLines -join "`n" | Set-Content -LiteralPath $OutMarkdown

if ($WriteBaseline) {
  Ensure-ParentDir -Path $BaselinePath
  $current | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $BaselinePath
}

Write-Output ($scoreboard | ConvertTo-Json -Depth 8)
