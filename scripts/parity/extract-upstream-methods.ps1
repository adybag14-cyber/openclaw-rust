param(
  [string]$UpstreamRepoPath = "..\\openclaw",
  [ValidateSet("base", "handlers")]
  [string]$Mode = "base",
  [string]$OutFile = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Extract-BaseMethods {
  param([string]$RepoPath)
  $methodsFile = Join-Path $RepoPath "src/gateway/server-methods-list.ts"
  if (-not (Test-Path -LiteralPath $methodsFile)) {
    throw "Upstream methods file not found: $methodsFile"
  }

  $lines = Get-Content -LiteralPath $methodsFile
  $start = $null
  for ($i = 0; $i -lt $lines.Length; $i++) {
    if ($lines[$i] -match '^const BASE_METHODS = \[$') {
      $start = $i + 1
      break
    }
  }
  if ($null -eq $start) {
    throw "Unable to locate BASE_METHODS in $methodsFile"
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
  return $methods
}

function Extract-HandlerMethods {
  param([string]$RepoPath)
  $handlersDir = Join-Path $RepoPath "src/gateway/server-methods"
  if (-not (Test-Path -LiteralPath $handlersDir)) {
    throw "Upstream handlers directory not found: $handlersDir"
  }

  $files = Get-ChildItem -LiteralPath $handlersDir -File -Filter "*.ts" |
    Where-Object { $_.Name -notlike "*.test.ts" }

  $methods = New-Object 'System.Collections.Generic.HashSet[string]'
  $simpleMethods = @("agent", "connect", "health", "poll", "send", "status", "wake")
  foreach ($file in $files) {
    $lines = Get-Content -LiteralPath $file.FullName
    foreach ($line in $lines) {
      $candidate = $null
      if ($line -match '^\s*"([A-Za-z0-9_.-]+)"\s*:\s*(?:async\s*)?(?:\(|[A-Za-z_][A-Za-z0-9_]*)') {
        $candidate = $matches[1]
      } elseif ($line -match '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(?:async\s*)?(?:\(|[A-Za-z_][A-Za-z0-9_]*)') {
        $candidate = $matches[1]
      }
      if (-not $candidate) {
        continue
      }
      if ($candidate.Contains(".") -or $candidate.Contains("-") -or $simpleMethods -contains $candidate) {
        [void]$methods.Add($candidate)
      }
    }
  }

  return @($methods)
}

$methods = switch ($Mode) {
  "base" { Extract-BaseMethods -RepoPath $UpstreamRepoPath; break }
  "handlers" { Extract-HandlerMethods -RepoPath $UpstreamRepoPath; break }
  default { throw "Unsupported mode: $Mode" }
}

$sorted = @($methods) | Sort-Object -Unique
$json = $sorted | ConvertTo-Json

if ($OutFile -and $OutFile.Trim().Length -gt 0) {
  $parent = Split-Path -Parent $OutFile
  if ($parent -and -not (Test-Path -LiteralPath $parent)) {
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
  }
  Set-Content -LiteralPath $OutFile -Value $json
}

Write-Output $json
