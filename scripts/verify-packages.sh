#!/usr/bin/env bash
set -euo pipefail

workspace_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$workspace_root"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

cargo package -p einstellung_derive --target-dir "$scratch/derive-target"

derive_crate="$(ls -t target/package/einstellung_derive-*.crate | head -n 1)"
mkdir -p "$scratch/derive-package"
tar -xzf "$derive_crate" -C "$scratch/derive-package"
derive_dir="$(find "$scratch/derive-package" -mindepth 1 -maxdepth 1 -type d -name 'einstellung_derive-*' -print -quit)"

if [[ -z "$derive_dir" ]]; then
    echo "failed to locate packaged einstellung_derive source" >&2
    exit 1
fi

# `cargo package` rewrites path dependencies as registry dependencies. Patch crates.io to the
# freshly packaged derive crate so the main package is verified against the exact derive artifact
# that would be published alongside it, rather than a previously published version.
cargo package -p einstellung \
    --target-dir "$scratch/einstellung-target" \
    --config "patch.crates-io.einstellung_derive.path=\"$derive_dir\""
