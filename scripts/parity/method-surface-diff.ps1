param(
  [string]$RustGatewayPath = "src/gateway.rs",
  [string]$UpstreamRepoPath = "..\\openclaw",
  [ValidateSet("base", "handlers", "both")]
  [string]$Surface = "both",
  [string]$OutDir = "parity/generated"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

if (-not (Test-Path -LiteralPath $OutDir)) {
  New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
}

$rustOut = Join-Path $OutDir "rust-methods.json"
$upstreamBaseOut = Join-Path $OutDir "upstream-methods.base.json"
$upstreamHandlersOut = Join-Path $OutDir "upstream-methods.handlers.json"
$diffOut = Join-Path $OutDir "method-surface-diff.json"
$reportOut = Join-Path "parity" "method-surface-report.md"

& (Join-Path $scriptDir "extract-rust-methods.ps1") -RustGatewayPath $RustGatewayPath -OutFile $rustOut | Out-Null
if ($Surface -eq "base" -or $Surface -eq "both") {
  & (Join-Path $scriptDir "extract-upstream-methods.ps1") -UpstreamRepoPath $UpstreamRepoPath -Mode "base" -OutFile $upstreamBaseOut | Out-Null
}
if ($Surface -eq "handlers" -or $Surface -eq "both") {
  & (Join-Path $scriptDir "extract-upstream-methods.ps1") -UpstreamRepoPath $UpstreamRepoPath -Mode "handlers" -OutFile $upstreamHandlersOut | Out-Null
}

$rustMethods = @(Get-Content -LiteralPath $rustOut -Raw | ConvertFrom-Json)

function Compare-Surfaces {
  param(
    [string[]]$UpstreamMethods,
    [string[]]$RustMethods
  )
  $rustSet = New-Object 'System.Collections.Generic.HashSet[string]'
  foreach ($method in $RustMethods) {
    [void]$rustSet.Add([string]$method)
  }
  $upstreamSet = New-Object 'System.Collections.Generic.HashSet[string]'
  foreach ($method in $UpstreamMethods) {
    [void]$upstreamSet.Add([string]$method)
  }

  $intersection = New-Object 'System.Collections.Generic.List[string]'
  foreach ($method in $UpstreamMethods) {
    if ($rustSet.Contains([string]$method)) {
      $intersection.Add([string]$method) | Out-Null
    }
  }
  $missingInRust = New-Object 'System.Collections.Generic.List[string]'
  foreach ($method in $UpstreamMethods) {
    if (-not $rustSet.Contains([string]$method)) {
      $missingInRust.Add([string]$method) | Out-Null
    }
  }
  $rustOnly = New-Object 'System.Collections.Generic.List[string]'
  foreach ($method in $RustMethods) {
    if (-not $upstreamSet.Contains([string]$method)) {
      $rustOnly.Add([string]$method) | Out-Null
    }
  }

  $upstreamCount = @($UpstreamMethods).Count
  $intersectionCount = @($intersection).Count
  $coverage = if ($upstreamCount -eq 0) { 0.0 } else { [Math]::Round(($intersectionCount * 100.0) / $upstreamCount, 2) }

  return [ordered]@{
    upstreamCount = $upstreamCount
    rustCount = @($RustMethods).Count
    intersectionCount = $intersectionCount
    coveragePercent = $coverage
    missingInRust = @($missingInRust)
    rustOnly = @($rustOnly)
  }
}

$surfaces = [ordered]@{}
if (Test-Path -LiteralPath $upstreamBaseOut) {
  $upstreamBase = @(Get-Content -LiteralPath $upstreamBaseOut -Raw | ConvertFrom-Json)
  $surfaces["base"] = Compare-Surfaces -UpstreamMethods $upstreamBase -RustMethods $rustMethods
}
if (Test-Path -LiteralPath $upstreamHandlersOut) {
  $upstreamHandlers = @(Get-Content -LiteralPath $upstreamHandlersOut -Raw | ConvertFrom-Json)
  $surfaces["handlers"] = Compare-Surfaces -UpstreamMethods $upstreamHandlers -RustMethods $rustMethods
}

$result = [ordered]@{
  generatedAtUtc = [DateTime]::UtcNow.ToString("o")
  rustGatewayPath = $RustGatewayPath
  upstreamRepoPath = $UpstreamRepoPath
  rustCount = @($rustMethods).Count
  surfaces = $surfaces
}

$result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $diffOut

function Format-List {
  param([string[]]$Items)
  if (@($Items).Count -eq 0) {
    return "- _None_"
  }
  return (@($Items) | ForEach-Object { "- ``$_``" }) -join "`n"
}

$sections = New-Object 'System.Collections.Generic.List[string]'
foreach ($surfaceName in $surfaces.Keys) {
  $surfaceInfo = $surfaces[$surfaceName]
  $section = @"
## Surface: $surfaceName

- Upstream method count: $($surfaceInfo.upstreamCount)
- Rust method count: $($surfaceInfo.rustCount)
- Shared methods: $($surfaceInfo.intersectionCount)
- Coverage vs upstream: $($surfaceInfo.coveragePercent)%

### Missing In Rust

$(Format-List -Items @($surfaceInfo.missingInRust))

### Rust-only Methods

$(Format-List -Items @($surfaceInfo.rustOnly))
"@
  $sections.Add($section) | Out-Null
}

$report = @"
# RPC Method Surface Diff

Generated (UTC): $($result.generatedAtUtc)

## Summary

- Rust method count: $($result.rustCount)
- Surfaces compared: $(@($surfaces.Keys).Count)

$($sections -join "`n`n")
"@

$report | Set-Content -LiteralPath $reportOut

Write-Output ($result | ConvertTo-Json -Depth 5)
