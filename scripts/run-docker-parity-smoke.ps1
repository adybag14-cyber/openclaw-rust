param(
    [string]$ImageTag = "openclaw-rs-parity-runtime:latest"
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Split-Path -Parent $scriptDir

docker build -f "$rootDir/deploy/Dockerfile.parity-runtime" -t $ImageTag $rootDir
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

docker run --rm --entrypoint /usr/local/bin/openclaw-agent-rs $ImageTag --help
exit $LASTEXITCODE
