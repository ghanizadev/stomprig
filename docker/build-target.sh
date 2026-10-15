#!/usr/bin/env bash
set -euo pipefail

readonly source_dir="/workspace/stomprig"
readonly local_source_dir="/source"
readonly package_dir="/tmp/stomprig-package"
readonly repository="${STOMPRIG_REPOSITORY:-https://github.com/ghanizadev/stomprig.git}"
readonly ref="${STOMPRIG_REF:-master}"
readonly source_mode="${STOMPRIG_SOURCE_MODE:-local}"
readonly target_platform="${STOMPRIG_TARGET_PLATFORM:?Dockerfile must set STOMPRIG_TARGET_PLATFORM}"

copy_local_source() {
    if [[ ! -f "${local_source_dir}/Cargo.toml" ]]; then
        echo "No Cargo.toml found in mounted source directory ${local_source_dir}." >&2
        return 1
    fi

    mkdir -p "${source_dir}"
    tar -C "${local_source_dir}" \
        --exclude=.git \
        --exclude=target \
        --exclude=dist \
        --exclude=macos-sdk \
        --exclude=.idea \
        -cf - . | tar -C "${source_dir}" -xf -
}

checkout_remote_source() {
    git clone --quiet --depth 1 --filter=blob:none --no-checkout \
        "${repository}" "${source_dir}"
    git -C "${source_dir}" sparse-checkout init --no-cone
    git -C "${source_dir}" sparse-checkout set \
        '/Cargo.toml' '/Cargo.lock' '/src/**' '/tests/**' \
        '/vendor/cpal/**' '/vendor/egui-desktop/**' '/vendor/vst3-host/**' \
        '/scripts/package-release.sh'
    git -C "${source_dir}" fetch --quiet --depth 1 origin "${ref}"
    git -C "${source_dir}" checkout --quiet --detach FETCH_HEAD
}

checkout_source() {
    rm -rf "${source_dir}"

    case "${source_mode}" in
        local)
            copy_local_source
            ;;
        remote)
            checkout_remote_source
            ;;
        *)
            echo "unsupported source mode: ${source_mode} (expected local or remote)" >&2
            return 2
            ;;
    esac

    find "${source_dir}" -type f -name '*.sh' -exec sed -i 's/\r$//' {} +
}

run_host_checks() {
    cd "${source_dir}"
    cargo fmt --all -- --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo check --locked --all-targets
    cargo test --locked --all-targets
}

package_archive() {
    local archive_name="$1"
    local archive_format="$2"

    mkdir -p /output
    rm -f "/output/${archive_name}"
    case "${archive_format}" in
        tar.gz)
            tar -czf "/output/${archive_name}" -C "${package_dir}" .
            ;;
        zip)
            (cd "${package_dir}" && zip -q -r "/output/${archive_name}" .)
            ;;
        *)
            echo "unsupported archive format: ${archive_format}" >&2
            return 1
            ;;
    esac
    echo "Created /output/${archive_name}"
}

build_linux() {
    run_host_checks
    cargo build --locked --release --bins
    STOMPRIG_PACKAGE_DIR="${package_dir}" \
        STOMPRIG_RELEASE_DIR="${CARGO_TARGET_DIR:-${source_dir}/target}/release" \
        STOMPRIG_SKIP_BUILD=1 \
        bash scripts/package-release.sh
    package_archive stomprig-linux.tar.gz tar.gz
}

build_windows() {
    run_host_checks
    local target="x86_64-pc-windows-msvc"
    local xwin_sysroot="${XWIN_CACHE_DIR:-/xwin-cache}/xwin"
    rustup target add "${target}"
    export CC_x86_64_pc_windows_msvc=clang-cl
    export CXX_x86_64_pc_windows_msvc=clang-cl
    export AR_x86_64_pc_windows_msvc=llvm-lib
    export CXXFLAGS_x86_64_pc_windows_msvc="--target=${target} -Wno-unused-command-line-argument -fuse-ld=lld-link /imsvc${xwin_sysroot}/crt/include /imsvc${xwin_sysroot}/sdk/include/ucrt /imsvc${xwin_sysroot}/sdk/include/um /imsvc${xwin_sysroot}/sdk/include/shared"
    export BINDGEN_EXTRA_CLANG_ARGS="--target=${target} -isystem ${xwin_sysroot}/crt/include -isystem ${xwin_sysroot}/sdk/include/ucrt -isystem ${xwin_sysroot}/sdk/include/um -isystem ${xwin_sysroot}/sdk/include/shared"
    export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER=wine
    cargo xwin test --locked --all-targets --target "${target}"
    cargo xwin build --locked --release --bins --target "${target}"
    STOMPRIG_PACKAGE_DIR="${package_dir}" \
        STOMPRIG_RELEASE_DIR="${CARGO_TARGET_DIR:-${source_dir}/target}/${target}/release" \
        STOMPRIG_BINARY_SUFFIX=.exe \
        STOMPRIG_VERIFY_RUNNER=wine \
        STOMPRIG_SKIP_BUILD=1 \
        bash scripts/package-release.sh
    local binary
    for binary in stomprig vst3-host-helper vst3-host-probe; do
        if ! file "${package_dir}/${binary}.exe" | grep --quiet 'PE32+'; then
            echo "${binary}.exe is not an x86_64 Windows executable" >&2
            return 1
        fi
    done
    package_archive stomprig-windows.zip zip
}

build_macos() {
    run_host_checks

    local sdk_archive
    sdk_archive="$(find /macos-sdk -maxdepth 1 -type f \( -name 'MacOSX*.sdk.tar.xz' -o -name 'MacOSX*.sdk.tar.bz2' \) -print -quit)"
    if [[ -z "${sdk_archive}" ]]; then
        echo "No MacOSX*.sdk.tar.xz or MacOSX*.sdk.tar.bz2 file found in /macos-sdk." >&2
        echo "Mount an Apple SDK archive with STOMPRIG_MACOS_SDK_DIR. See docker/README.md." >&2
        return 1
    fi

    mkdir -p /opt/osxcross/tarballs
    local sdk_checksum
    sdk_checksum="$(sha256sum "${sdk_archive}" | cut -d ' ' -f 1)"
    local osxcross_revision
    osxcross_revision="$(git -C /opt/osxcross rev-parse HEAD)"
    local toolchain_identity="${sdk_checksum}-${osxcross_revision}"
    local sdk_marker="/opt/osxcross/target/.stomprig-sdk-sha256"
    if [[ ! -x /opt/osxcross/target/bin/o64-clang || \
          ! -x /opt/osxcross/target/bin/oa64-clang || \
          ! -f "${sdk_marker}" || "$(cat "${sdk_marker}")" != "${toolchain_identity}" ]]; then
        mkdir -p /opt/osxcross/target
        find /opt/osxcross/target -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
        cp "${sdk_archive}" /opt/osxcross/tarballs/
        (
            cd /opt/osxcross
            UNATTENDED=1 BUILD_FLAVOR=llvm ENABLE_ARCHS="x86_64 arm64" \
                TARGET_DIR=/opt/osxcross/target ./build.sh
        )
        printf '%s\n' "${toolchain_identity}" > "${sdk_marker}"
    fi

    export PATH="/opt/osxcross/target/bin:${PATH}"
    export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-10.13}"
    export CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=o64-clang
    export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=oa64-clang
    export CC_x86_64_apple_darwin=o64-clang
    export CC_aarch64_apple_darwin=oa64-clang

    local target
    for target in x86_64-apple-darwin aarch64-apple-darwin; do
        rustup target add "${target}"
        cargo test --locked --all-targets --no-run --target "${target}"
        cargo build --locked --release --bins --target "${target}"
    done

    local universal_dir="/tmp/stomprig-macos-universal"
    mkdir -p "${universal_dir}"
    local binary
    for binary in stomprig vst3-host-helper vst3-host-probe; do
        lipo -create \
            "${CARGO_TARGET_DIR:-${source_dir}/target}/x86_64-apple-darwin/release/${binary}" \
            "${CARGO_TARGET_DIR:-${source_dir}/target}/aarch64-apple-darwin/release/${binary}" \
            -output "${universal_dir}/${binary}"
        lipo -verify_arch x86_64 arm64 "${universal_dir}/${binary}"
    done

    STOMPRIG_PACKAGE_DIR="${package_dir}" \
        STOMPRIG_RELEASE_DIR="${universal_dir}" \
        STOMPRIG_SKIP_PACKAGE_VERIFY=1 \
        STOMPRIG_SKIP_BUILD=1 \
        bash scripts/package-release.sh
    package_archive stomprig-macos-universal.tar.gz tar.gz
}

checkout_source
cd "${source_dir}"

case "${target_platform}" in
    linux)
        build_linux
        ;;
    windows)
        build_windows
        ;;
    macos)
        build_macos
        ;;
    *)
        echo "unsupported target platform: ${target_platform}" >&2
        exit 2
        ;;
esac
