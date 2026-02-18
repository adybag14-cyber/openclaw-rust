param()

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Split-Path -Parent $scriptDir
$composeFile = Join-Path $rootDir "deploy/docker-compose.parity.yml"

$exitCode = 0
$didUp = $false
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

    $didUp = $true
    docker compose -f $composeFile up --abort-on-container-exit --exit-code-from assertor
    if ($LASTEXITCODE -ne 0) {
        $exitCode = $LASTEXITCODE
        throw "docker compose up failed"
    }
}
finally {
    if ($didUp) {
        docker compose -f $composeFile down --volumes --remove-orphans 2>$null | Out-Null
    }
    if ($exitCode -ne 0) {
        exit $exitCode
    }
}
