#!/usr/bin/env bash
set -euo pipefail

workspace_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$workspace_root"

scratch="$(mktemp -d)"
export CARGO_TARGET_DIR="$scratch/target"
lock_backup="$scratch/Cargo.lock"
cp Cargo.lock "$lock_backup"
cleanup() {
    cp "$lock_backup" Cargo.lock
    rm -rf "$scratch"
}
trap cleanup EXIT

# Fail before packaging if the checked-in lockfile is already stale. The main package check below
# uses a temporary crates.io patch, which Cargo records in Cargo.lock; cleanup restores the exact
# checked-in lockfile afterward.
cargo metadata --locked --no-deps --format-version 1 >/dev/null

cargo package -p einstellung_derive

derive_crate="$(ls -t "$CARGO_TARGET_DIR"/package/einstellung_derive-*.crate | head -n 1)"
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
    --config "patch.crates-io.einstellung_derive.path=\"$derive_dir\""
