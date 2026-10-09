#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package_dir="${RACK_PACKAGE_DIR:-${project_root}/dist/rack}"
release_dir="${project_root}/target/release"

cargo build --release --bins --manifest-path "${project_root}/Cargo.toml"

mkdir -p "${package_dir}"
for binary in rack vst3-host-helper vst3-host-probe; do
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

echo "Packaged rack, vst3-host-helper, and vst3-host-probe in ${package_dir}"
