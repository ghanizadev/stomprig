# Local container builds

The builders use a read-only mount of the current checkout by default, so uncommitted changes are included. Each container copies the source into its own workspace and excludes `.git`, `target/`, `dist/`, the local macOS SDK directory, and `.idea/` from that copy. To build a remote ref instead, set `STOMPRIG_SOURCE_MODE=remote`; the builder then clones `STOMPRIG_REPOSITORY` and checks out `STOMPRIG_REF` using Git sparse checkout.

## Build with just

Install [just](https://just.systems/) and Docker Compose, then run commands from the repository root:

```text
just                 # list available recipes
just build           # build all three release packages sequentially
just build-linux
just build-windows
just build-macos
```

The packages are written to `dist/`. These recipes use the same Compose builders described below and build the current working tree. For a remote build, set `STOMPRIG_SOURCE_MODE=remote` together with optional `STOMPRIG_REPOSITORY` and `STOMPRIG_REF` values. The macOS recipe requires an Apple SDK archive as described in the next section.

Run one platform from the repository root:

```powershell
docker compose run --build --rm linux
docker compose run --build --rm windows
docker compose run --build --rm macos
```

On Windows, build all three sequentially with `scripts/build-containers.ps1`. Select one with `-Platform linux`, `-Platform windows`, or `-Platform macos`.

The archives are written to `dist/` on the host. All builders use an x86_64 Linux image so target output is consistent across hosts. Windows builds x86_64 MSVC. macOS builds a universal x86_64 and arm64 archive. ARM Docker hosts need `linux/amd64` emulation enabled.

To build a remote branch or tag other than `master`, set `STOMPRIG_SOURCE_MODE=remote` and `STOMPRIG_REF`. To use another remote repository, also set `STOMPRIG_REPOSITORY`.

## macOS SDK

Docker runs Linux containers on Windows, macOS, and Linux. The macOS target therefore uses OSXCross and needs an Apple macOS SDK archive that you are licensed to use. Docker does not include this SDK. Put a file named like `MacOSX14.5.sdk.tar.xz` in `macos-sdk/`, or set `STOMPRIG_MACOS_SDK_DIR` to a directory containing that archive. The container builds the cross-toolchain from the mounted SDK.

In PowerShell, point the mount at an SDK directory with:

```powershell
$env:STOMPRIG_MACOS_SDK_DIR = "D:\SDKs\macos"
```

The archive must contain an SDK that supports both x86_64 and arm64. OSXCross documents how to package an SDK from Xcode in its [SDK instructions](https://github.com/tpoechtrager/osxcross/blob/master/README.SDK.md).

The macOS builder compiles test targets for both macOS architectures. macOS test binaries cannot run inside the Linux container, so the shared test suite also runs natively in that container before cross compilation.

The Windows builder uses `cargo-xwin` and Wine to run the Windows tests and verify the packaged helper executables. Its unit test suite also runs natively in the Linux container. Linux builds and runs its tests natively.

To run the full matrix, invoke all three Compose commands above. The Windows and macOS output can be built from any Docker host; the macOS SDK is the only extra local input.
