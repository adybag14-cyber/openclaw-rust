param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$CommandLine
)

$candidateBins = @()

if ($env:OPENCLAW_MINGW_BIN) {
    $candidateBins += $env:OPENCLAW_MINGW_BIN
}

try {
    $wingetBins = Get-ChildItem -Path "C:\Users\$env:USERNAME\AppData\Local\Microsoft\WinGet\Packages" -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like "BrechtSanders.WinLibs*" } |
        ForEach-Object { Join-Path $_.FullName "mingw64\bin" }
    $candidateBins += $wingetBins
} catch {
}

$candidateBins += "C:\Users\$env:USERNAME\gcc\bin"
$candidateBins += "C:\Users\Public\gcc\bin"

try {
    $userBins = Get-ChildItem -Path "C:\Users" -Directory -ErrorAction SilentlyContinue |
        ForEach-Object { Join-Path $_.FullName "gcc\bin" }
    $candidateBins += $userBins
} catch {
}

$mingwBin = $null
foreach ($candidate in ($candidateBins | Where-Object { $_ } | Select-Object -Unique)) {
    $gccPath = Join-Path $candidate "gcc.exe"
    $arPath = Join-Path $candidate "ar.exe"
    if ((Test-Path $gccPath) -and (Test-Path $arPath)) {
        $mingwBin = $candidate
        break
    }
}

if (-not $mingwBin) {
    Write-Error "Missing MinGW toolchain. Set OPENCLAW_MINGW_BIN or install WinLibs (needs gcc.exe and ar.exe)."
    exit 1
}

Write-Output "Using MinGW bin: $mingwBin"

$env:PATH = "$mingwBin;$env:PATH"

cmd.exe /d /s /c $CommandLine
exit $LASTEXITCODE
