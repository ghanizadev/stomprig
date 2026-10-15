[CmdletBinding()]
param(
    [ValidateSet("all", "linux", "windows", "macos")]
    [string]$Platform = "all"
)

$ErrorActionPreference = "Stop"
$RepositoryRoot = Split-Path -Parent $PSScriptRoot
$OutputDirectory = Join-Path $RepositoryRoot "dist"

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null

$Platforms = if ($Platform -eq "all") {
    @("linux", "windows", "macos")
} else {
    @($Platform)
}

foreach ($CurrentPlatform in $Platforms) {
    Write-Host "Building $CurrentPlatform package..."
    & docker compose --project-directory $RepositoryRoot run --build --rm --no-deps $CurrentPlatform
    if ($LASTEXITCODE -ne 0) {
        throw "The $CurrentPlatform Docker build failed with exit code $LASTEXITCODE."
    }
}

Write-Host "Packages are in $OutputDirectory"
