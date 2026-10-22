default:
    @just --list

[windows]
set shell := ["powershell.exe", "-NoLogo", "-Command"]

distribution_runner := if os() == "windows" { "powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File" } else { "pwsh -NoLogo -NoProfile -File" }

# Create Velopack distributions from the versioned local builds. Linux needs a PNG icon;
# macOS needs an ICNS icon and must be packaged on macOS.
distribution-windows version:
    {{distribution_runner}} scripts/create-distributions.ps1 -Version "{{version}}" -Platform windows

distribution-linux version icon:
    {{distribution_runner}} scripts/create-distributions.ps1 -Version "{{version}}" -Platform linux -LinuxIcon "{{icon}}"

distribution-macos version icon:
    {{distribution_runner}} scripts/create-distributions.ps1 -Version "{{version}}" -Platform macos -MacIcon "{{icon}}"

# Build and package all three distributions. Run this recipe on macOS to produce the .pkg.
distributions version linux_icon macos_icon:
    {{distribution_runner}} scripts/create-distributions.ps1 -Version "{{version}}" -Platform all -LinuxIcon "{{linux_icon}}" -MacIcon "{{macos_icon}}"

# Build release packages from the current checkout for Linux, Windows, and universal macOS.
build: build-all

# Run the platform builders in order to avoid competing Docker builds.
build-all: build-linux build-windows build-macos

# Build the Linux x86_64 release package into dist/.
build-linux:
    docker compose run --build --rm --no-deps linux

# Build the Windows x86_64 MSVC release package into dist/.
build-windows:
    docker compose run --build --rm --no-deps windows

# Build the universal x86_64 + arm64 macOS release package into dist/.
# Requires an Apple macOS SDK archive; see docker/README.md.
build-macos:
    docker compose run --build --rm --no-deps macos
