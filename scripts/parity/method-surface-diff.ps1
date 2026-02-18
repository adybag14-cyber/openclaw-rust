param(
  [string]$RustGatewayPath = "src/gateway.rs",
  [string]$UpstreamRepoPath = "..\\openclaw",
  [string]$OutDir = "parity/generated"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

if (-not (Test-Path -LiteralPath $OutDir)) {
  New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
}

$rustOut = Join-Path $OutDir "rust-methods.json"
$upstreamOut = Join-Path $OutDir "upstream-methods.json"
$diffOut = Join-Path $OutDir "method-surface-diff.json"
$reportOut = Join-Path "parity" "method-surface-report.md"

& (Join-Path $scriptDir "extract-rust-methods.ps1") -RustGatewayPath $RustGatewayPath -OutFile $rustOut | Out-Null
& (Join-Path $scriptDir "extract-upstream-methods.ps1") -UpstreamRepoPath $UpstreamRepoPath -OutFile $upstreamOut | Out-Null

$rustMethods = @(Get-Content -LiteralPath $rustOut -Raw | ConvertFrom-Json)
$upstreamMethods = @(Get-Content -LiteralPath $upstreamOut -Raw | ConvertFrom-Json)

$rustSet = New-Object 'System.Collections.Generic.HashSet[string]'
foreach ($method in $rustMethods) {
  [void]$rustSet.Add([string]$method)
}
$upstreamSet = New-Object 'System.Collections.Generic.HashSet[string]'
foreach ($method in $upstreamMethods) {
  [void]$upstreamSet.Add([string]$method)
}

$intersection = New-Object 'System.Collections.Generic.List[string]'
foreach ($method in $upstreamMethods) {
  if ($rustSet.Contains([string]$method)) {
    $intersection.Add([string]$method) | Out-Null
  }
}
$missingInRust = New-Object 'System.Collections.Generic.List[string]'
foreach ($method in $upstreamMethods) {
  if (-not $rustSet.Contains([string]$method)) {
    $missingInRust.Add([string]$method) | Out-Null
  }
}
$rustOnly = New-Object 'System.Collections.Generic.List[string]'
foreach ($method in $rustMethods) {
  if (-not $upstreamSet.Contains([string]$method)) {
    $rustOnly.Add([string]$method) | Out-Null
  }
}

$upstreamCount = @($upstreamMethods).Count
$intersectionCount = @($intersection).Count
$coverage = if ($upstreamCount -eq 0) { 0.0 } else { [Math]::Round(($intersectionCount * 100.0) / $upstreamCount, 2) }

$result = [ordered]@{
  generatedAtUtc = [DateTime]::UtcNow.ToString("o")
  rustGatewayPath = $RustGatewayPath
  upstreamRepoPath = $UpstreamRepoPath
  upstreamMethodsFile = "src/gateway/server-methods-list.ts (BASE_METHODS)"
  rustCount = @($rustMethods).Count
  upstreamCount = $upstreamCount
  intersectionCount = $intersectionCount
  coveragePercent = $coverage
  missingInRust = @($missingInRust)
  rustOnly = @($rustOnly)
}

$result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $diffOut

$missingLines = if (@($missingInRust).Count -eq 0) {
  "- _None_"
} else {
  (@($missingInRust) | ForEach-Object { "- ``$_``" }) -join "`n"
}
$rustOnlyLines = if (@($rustOnly).Count -eq 0) {
  "- _None_"
} else {
  (@($rustOnly) | ForEach-Object { "- ``$_``" }) -join "`n"
}

$report = @"
# RPC Method Surface Diff

Generated (UTC): $($result.generatedAtUtc)

## Summary

- Upstream method count: $($result.upstreamCount)
- Rust method count: $($result.rustCount)
- Shared methods: $($result.intersectionCount)
- Coverage vs upstream: $($result.coveragePercent)%

## Missing In Rust

$missingLines

## Rust-only Methods

$rustOnlyLines
"@

$report | Set-Content -LiteralPath $reportOut

Write-Output ($result | ConvertTo-Json -Depth 5)
