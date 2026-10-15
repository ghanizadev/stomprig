#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package_dir="${STOMPRIG_PACKAGE_DIR:-${project_root}/dist/stomprig}"
release_dir="${project_root}/target/release"
if [[ -n "${STOMPRIG_RELEASE_DIR:-}" ]]; then
    release_dir="${STOMPRIG_RELEASE_DIR}"
fi
binary_suffix="${STOMPRIG_BINARY_SUFFIX:-}"
if [[ -z "${binary_suffix}" && "${OS:-}" == "Windows_NT" ]]; then
    binary_suffix=".exe"
fi

if [[ "${STOMPRIG_SKIP_BUILD:-0}" != "1" ]]; then
    cargo build --locked --release --bins --manifest-path "${project_root}/Cargo.toml"
fi

mkdir -p "${package_dir}"
for binary in stomprig vst3-host-helper vst3-host-probe; do
    source="${release_dir}/${binary}${binary_suffix}"
    if [[ ! -f "${source}" ]]; then
        echo "missing packaged binary: ${source}" >&2
        exit 1
    fi
    cp "${source}" "${package_dir}/"
done

application="${package_dir}/stomprig"
application="${application}${binary_suffix}"
if [[ "${STOMPRIG_SKIP_PACKAGE_VERIFY:-0}" != "1" ]]; then
    if [[ -n "${STOMPRIG_VERIFY_RUNNER:-}" ]]; then
        "${STOMPRIG_VERIFY_RUNNER}" "${application}" --stomprig-verify-vst3-tools
    elif ! "${application}" --stomprig-verify-vst3-tools; then
        echo "packaged application could not find or launch its VST3 helper/probe executables" >&2
        exit 1
    fi
fi

echo "Packaged StompRig, vst3-host-helper, and vst3-host-probe in ${package_dir}"
