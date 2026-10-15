default:
    @just --list

[windows]
set shell := ["powershell.exe", "-NoLogo", "-Command"]

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
