param(
  [string]$UpstreamRepoPath = "..\\openclaw",
  [switch]$WriteBaseline
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

& (Join-Path $scriptDir "method-surface-diff.ps1") -Surface both -UpstreamRepoPath $UpstreamRepoPath | Out-Null
if ($WriteBaseline) {
  & (Join-Path $scriptDir "build-scoreboard.ps1") -WriteBaseline | Out-Null
} else {
  & (Join-Path $scriptDir "build-scoreboard.ps1") | Out-Null
}
& (Join-Path $scriptDir "run-replay-corpus.ps1") | Out-Null

Write-Output "[parity] CP0 gate passed"
Write-Output "[parity] scoreboard: parity/generated/parity-scoreboard.md"
