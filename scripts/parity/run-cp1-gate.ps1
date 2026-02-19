param(
  [string]$CargoCommand = "cargo",
  [string]$Toolchain = "1.83.0-x86_64-pc-windows-gnu"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$tests = @(
  "gateway_server::tests::standalone_gateway_serves_control_plane_rpcs_without_upstream_runtime",
  "gateway_server::tests::standalone_gateway_authz_matrix_enforces_roles_and_scopes",
  "gateway_server::tests::broadcaster_backpressure_drop_if_slow_semantics"
)

$toolchainArg = if ($Toolchain -and $Toolchain.Trim().Length -gt 0) {
  "+$($Toolchain.Trim())"
} else {
  ""
}

foreach ($testName in $tests) {
  Write-Output "[parity] running CP1 fixture: $testName"
  if ($toolchainArg) {
    & $CargoCommand $toolchainArg test $testName -- --nocapture
  } else {
    & $CargoCommand test $testName -- --nocapture
  }
  if ($LASTEXITCODE -ne 0) {
    throw "CP1 fixture failed: $testName (exit $LASTEXITCODE)"
  }
}

Write-Output "[parity] CP1 gate passed"
