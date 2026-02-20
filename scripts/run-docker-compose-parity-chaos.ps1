param(
    [int]$RestartDelaySecs = 3
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Split-Path -Parent $scriptDir
$composeFile = Join-Path $rootDir "deploy/docker-compose.parity-chaos.yml"

$exitCode = 0
$didUp = $false
$restartJob = $null
try {
    docker info | Out-Null
    if ($LASTEXITCODE -ne 0) {
        $exitCode = $LASTEXITCODE
        throw "docker daemon is not reachable. Start Docker Desktop/service and retry."
    }

    docker compose -f $composeFile build
    if ($LASTEXITCODE -ne 0) {
        $exitCode = $LASTEXITCODE
        throw "docker compose build failed"
    }

    $restartJob = Start-Job -ScriptBlock {
        param($ComposePath, $Delay)
        Start-Sleep -Seconds $Delay
        docker compose -f $ComposePath restart rust-agent | Out-Null
    } -ArgumentList $composeFile, $RestartDelaySecs

    $didUp = $true
    docker compose -f $composeFile up --abort-on-container-exit --exit-code-from assertor
    if ($LASTEXITCODE -ne 0) {
        $exitCode = $LASTEXITCODE
        throw "docker compose chaos run failed"
    }
}
finally {
    if ($restartJob) {
        Wait-Job -Job $restartJob -ErrorAction SilentlyContinue | Out-Null
        Remove-Job -Job $restartJob -Force -ErrorAction SilentlyContinue | Out-Null
    }
    if ($didUp) {
        docker compose -f $composeFile down --volumes --remove-orphans 2>$null | Out-Null
    }
    if ($exitCode -ne 0) {
        exit $exitCode
    }
}
