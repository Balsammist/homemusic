#!/usr/bin/env bash
set -euo pipefail

native_dir=$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")
export HOMEMUSIC_ROOT="${HOMEMUSIC_ROOT:-$(dirname -- "$native_dir")}"
binary="${CARGO_TARGET_DIR:-$native_dir/target}/release/homemusic-native"
if [[ ! -x "$binary" ]]; then
    printf 'Build first: cargo build --release --locked --manifest-path "%s/Cargo.toml"\n' "$native_dir" >&2
    exit 1
fi
exec "$binary" "$@"
