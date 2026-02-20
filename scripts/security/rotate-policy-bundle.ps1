param(
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$ArgsList
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
& python (Join-Path $scriptDir "rotate-policy-bundle.py") @ArgsList
