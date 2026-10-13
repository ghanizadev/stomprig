#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package_dir="${STOMPRIG_PACKAGE_DIR:-${project_root}/dist/stomprig}"
release_dir="${project_root}/target/release"

cargo build --release --bins --manifest-path "${project_root}/Cargo.toml"

mkdir -p "${package_dir}"
for binary in stomprig vst3-host-helper vst3-host-probe; do
    source="${release_dir}/${binary}"
    if [[ "${OS:-}" == "Windows_NT" ]]; then
        source="${source}.exe"
    fi
    if [[ ! -f "${source}" ]]; then
        echo "missing packaged binary: ${source}" >&2
        exit 1
    fi
    cp "${source}" "${package_dir}/"
done

application="${package_dir}/stomprig"
if [[ "${OS:-}" == "Windows_NT" ]]; then
    application="${application}.exe"
fi
if ! "${application}" --stomprig-verify-vst3-tools; then
    echo "packaged application could not find or launch its VST3 helper/probe executables" >&2
    exit 1
fi

echo "Packaged StompRig, vst3-host-helper, and vst3-host-probe in ${package_dir}"
