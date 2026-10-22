[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [ValidateSet("windows", "linux", "macos", "all")]
    [string]$Platform,

    [string]$LinuxIcon,
    [string]$MacIcon
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($Version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$') {
    throw "Version '$Version' must be stable SemVer (MAJOR.MINOR.PATCH with optional build metadata; prereleases are excluded)."
}

$platforms = if ($Platform -eq "all") {
    @("windows", "linux", "macos")
} else {
    @($Platform)
}

$isMacOS = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::OSX
)
if ($platforms -contains "macos" -and -not $isMacOS) {
    throw "Velopack macOS packages must be created on macOS. Run this recipe on a Mac."
}

if ($platforms -contains "linux") {
    if ([string]::IsNullOrWhiteSpace($LinuxIcon) -or
        [System.IO.Path]::GetExtension($LinuxIcon) -ne ".png" -or
        -not (Test-Path -LiteralPath $LinuxIcon -PathType Leaf)) {
        throw "Linux distribution requires an existing PNG icon path."
    }
}
if ($platforms -contains "macos") {
    if ([string]::IsNullOrWhiteSpace($MacIcon) -or
        [System.IO.Path]::GetExtension($MacIcon) -ne ".icns" -or
        -not (Test-Path -LiteralPath $MacIcon -PathType Leaf)) {
        throw "macOS distribution requires an existing ICNS icon path."
    }
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$distRoot = Join-Path $repositoryRoot "dist"
$toolManifest = Join-Path $repositoryRoot ".config/dotnet-tools.json"
$archiveNames = @{
    windows = "stomprig-windows.zip"
    linux   = "stomprig-linux.tar.gz"
    macos   = "stomprig-macos-universal.tar.gz"
}
$velopackTargets = @{
    windows = @{ Directive = "[win]"; PackId = "stomprig-win"; Channel = "win"; MainExe = "stomprig.exe" }
    linux   = @{ Directive = "[linux]"; PackId = "stomprig-linux"; Channel = "linux"; MainExe = "stomprig" }
    macos   = @{ Directive = "[osx]"; PackId = "stomprig-osx"; Channel = "osx"; MainExe = "stomprig" }
}

Push-Location $repositoryRoot
try {
    foreach ($targetPlatform in $platforms) {
        Write-Host "Building $targetPlatform StompRig $Version..."
        & docker compose run --build --rm --no-deps --env "STOMPRIG_BUILD_VERSION=$Version" $targetPlatform
        if ($LASTEXITCODE -ne 0) {
            throw "Docker build for $targetPlatform failed with exit code $LASTEXITCODE."
        }
    }

    & dotnet tool restore --tool-manifest $toolManifest
    if ($LASTEXITCODE -ne 0) {
        throw "Could not restore the pinned Velopack CLI tool. Install the .NET 8 SDK and retry."
    }

    foreach ($targetPlatform in $platforms) {
        $archivePath = Join-Path $distRoot $archiveNames[$targetPlatform]
        if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
            throw "Expected build archive was not created: $archivePath"
        }

        $inputDirectory = [System.IO.Path]::GetFullPath(
            (Join-Path $distRoot ".velopack-input/$targetPlatform")
        )
        $distPrefix = [System.IO.Path]::GetFullPath($distRoot).TrimEnd(
            [System.IO.Path]::DirectorySeparatorChar,
            [System.IO.Path]::AltDirectorySeparatorChar
        ) + [System.IO.Path]::DirectorySeparatorChar
        if (-not $inputDirectory.StartsWith(
                $distPrefix,
                [System.StringComparison]::OrdinalIgnoreCase
            )) {
            throw "Refusing to prepare a package outside the dist directory: $inputDirectory"
        }
        if (Test-Path -LiteralPath $inputDirectory) {
            Remove-Item -LiteralPath $inputDirectory -Recurse -Force
        }
        New-Item -ItemType Directory -Path $inputDirectory -Force | Out-Null

        if ($targetPlatform -eq "windows") {
            Expand-Archive -LiteralPath $archivePath -DestinationPath $inputDirectory
        } else {
            & tar -xzf $archivePath -C $inputDirectory
            if ($LASTEXITCODE -ne 0) {
                throw "Could not extract the $targetPlatform build archive."
            }
        }

        $target = $velopackTargets[$targetPlatform]
        $vpkArguments = @(
            $target.Directive,
            "pack",
            "--packId", $target.PackId,
            "--packVersion", $Version,
            "--packDir", $inputDirectory,
            "--mainExe", $target.MainExe,
            "--packTitle", "StompRig",
            "--channel", $target.Channel,
            "--outputDir", (Join-Path $distRoot "velopack")
        )
        if ($targetPlatform -eq "linux") {
            $vpkArguments += @("--icon", [System.IO.Path]::GetFullPath($LinuxIcon))
        } elseif ($targetPlatform -eq "macos") {
            $vpkArguments += @("--icon", [System.IO.Path]::GetFullPath($MacIcon))
        }

        Write-Host "Creating Velopack $targetPlatform distribution for $Version..."
        & dotnet tool run vpk -- @vpkArguments
        if ($LASTEXITCODE -ne 0) {
            throw "Velopack packaging for $targetPlatform failed with exit code $LASTEXITCODE."
        }
    }

    Write-Host "Velopack distributions are in $distRoot/velopack"
} finally {
    Pop-Location
}
