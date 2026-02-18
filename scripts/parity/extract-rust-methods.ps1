param(
  [string]$RustGatewayPath = "src/gateway.rs",
  [string]$OutFile = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $RustGatewayPath)) {
  throw "Rust gateway file not found: $RustGatewayPath"
}

$lines = Get-Content -LiteralPath $RustGatewayPath
$start = $null
for ($i = 0; $i -lt $lines.Length; $i++) {
  if ($lines[$i] -match '^const SUPPORTED_RPC_METHODS: &\[&str\] = &\[$') {
    $start = $i + 1
    break
  }
}
if ($null -eq $start) {
  throw "Unable to locate SUPPORTED_RPC_METHODS in $RustGatewayPath"
}

$methods = New-Object 'System.Collections.Generic.List[string]'
for ($i = $start; $i -lt $lines.Length; $i++) {
  $line = $lines[$i]
  if ($line -match '^\];\s*$') {
    break
  }
  if ($line -match '^\s*"([^"]+)",\s*$') {
    $methods.Add($matches[1]) | Out-Null
  }
}

$sorted = $methods | Sort-Object -Unique
$json = $sorted | ConvertTo-Json

if ($OutFile -and $OutFile.Trim().Length -gt 0) {
  $parent = Split-Path -Parent $OutFile
  if ($parent -and -not (Test-Path -LiteralPath $parent)) {
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
  }
  Set-Content -LiteralPath $OutFile -Value $json
}

Write-Output $json
