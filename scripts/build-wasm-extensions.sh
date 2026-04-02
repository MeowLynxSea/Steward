#!/usr/bin/env bash
# Build all WASM tools from source.
#
# Verifies that every registry tool compiles against the
# current WIT definitions. Used by CI and can be run locally.
#
# Prerequisites:
#   rustup target add wasm32-wasip2
#   cargo install cargo-component --locked
#
# Usage:
#   ./scripts/build-wasm-extensions.sh

set -euo pipefail

cd "$(dirname "$0")/.."

FAILED=()

build_extension() {
    local manifest_path="$1"
    local source_dir
    local crate_name

    source_dir=$(jq -r '.source.dir' "$manifest_path")
    crate_name=$(jq -r '.source.crate_name' "$manifest_path")
    local name
    name=$(basename "$manifest_path" .json)

    if [ ! -d "$source_dir" ]; then
        echo "  SKIP $name (source dir $source_dir not found)"
        return 0
    fi

    echo "  BUILD $name ($crate_name) from $source_dir"
    if ! cargo component build --release --manifest-path "$source_dir/Cargo.toml" 2>&1; then
        echo "  FAIL $name"
        FAILED+=("$name")
        return 1
    fi
    echo "  OK   $name"
}

echo "Building WASM tools..."
for manifest in registry/tools/*.json; do
    build_extension "$manifest" || true
done

echo ""
if [ ${#FAILED[@]} -gt 0 ]; then
    echo "FAILED: ${FAILED[*]}"
    exit 1
else
    echo "All WASM tools built successfully."
fi
