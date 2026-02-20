param(
  [string]$CargoCommand = "cargo",
  [string]$Toolchain = "1.83.0-x86_64-pc-windows-gnu"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
  $PSNativeCommandUseErrorActionPreference = $false
}

function Invoke-CargoTest {
  param([string]$Filter)
  $toolchainArg = if ($Toolchain -and $Toolchain.Trim().Length -gt 0) {
    "+$($Toolchain.Trim())"
  } else {
    ""
  }
  Write-Output "[parity] running replay test: $Filter"
  if ($toolchainArg) {
    & $CargoCommand $toolchainArg test $Filter -- --nocapture
  } else {
    & $CargoCommand test $Filter -- --nocapture
  }
  if ($LASTEXITCODE -ne 0) {
    throw "Replay corpus test failed for filter '$Filter' with exit code $LASTEXITCODE"
  }
}

Invoke-CargoTest -Filter "protocol_corpus_snapshot_matches_expectations"
Invoke-CargoTest -Filter "dispatcher_payload_corpus_matches_upstream_fixtures"

Write-Output "[parity] replay corpus suite passed"

