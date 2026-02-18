param(
  [string]$TestFilter = "dispatcher_payload_corpus_matches_upstream_fixtures",
  [string]$CargoCommand = "cargo",
  [string]$Toolchain = "1.83.0-x86_64-pc-windows-gnu"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath "tests/parity/gateway-payload-corpus.json")) {
  throw "Payload corpus file not found: tests/parity/gateway-payload-corpus.json"
}

Write-Output "[parity] running payload-shape harness: $TestFilter"
$toolchainArg = if ($Toolchain -and $Toolchain.Trim().Length -gt 0) {
  "+$($Toolchain.Trim())"
} else {
  ""
}
if ($toolchainArg) {
  & $CargoCommand $toolchainArg test $TestFilter -- --nocapture
} else {
  & $CargoCommand test $TestFilter -- --nocapture
}
if ($LASTEXITCODE -ne 0) {
  throw "Payload-shape harness failed with exit code $LASTEXITCODE"
}

Write-Output "[parity] payload-shape harness passed"
