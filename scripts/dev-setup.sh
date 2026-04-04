#!/usr/bin/env bash
# Developer setup script for Steward.
#
# Gets a fresh checkout ready for development without requiring
# Docker, PostgreSQL, or any external services.
#
# Usage:
#   ./scripts/dev-setup.sh

set -euo pipefail

cd "$(dirname "$0")/.."

echo "=== Steward Developer Setup ==="
echo ""

if ! command -v rustup &>/dev/null; then
    echo "ERROR: rustup not found. Install from https://rustup.rs"
    exit 1
fi
echo "[1/8] rustup found: $(rustup --version 2>/dev/null | head -1)"

if ! command -v npm &>/dev/null; then
    echo "ERROR: npm not found. Install Node.js 20+ before running this bootstrap."
    exit 1
fi
echo "[2/8] npm found: $(npm --version 2>/dev/null)"

echo "[3/8] Adding wasm32-wasip2 target..."
rustup target add wasm32-wasip2

echo "[4/8] Installing wasm-tools..."
if command -v wasm-tools &>/dev/null; then
    echo "  wasm-tools already installed: $(wasm-tools --version)"
else
    cargo install wasm-tools --locked
fi

echo "[5/8] Installing UI dependencies..."
npm --prefix ui ci

echo "[6/8] Building UI bundle..."
npm --prefix ui run build

echo "[7/8] Running cargo check..."
cargo check

echo "[8/8] Installing git hooks..."
HOOKS_DIR=$(git rev-parse --git-path hooks 2>/dev/null) || true
if [ -n "$HOOKS_DIR" ]; then
    mkdir -p "$HOOKS_DIR"
    SCRIPTS_ABS="$(cd "$(dirname "$0")" && pwd)"
    ln -sf "$SCRIPTS_ABS/commit-msg-regression.sh" "$HOOKS_DIR/commit-msg"
    echo "  commit-msg hook installed (regression test enforcement)"
    ln -sf "$SCRIPTS_ABS/pre-commit-safety.sh" "$HOOKS_DIR/pre-commit"
    echo "  pre-commit hook installed (UTF-8, case-sensitivity, /tmp, redaction checks)"
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    ln -sf "$REPO_ROOT/.githooks/pre-push" "$HOOKS_DIR/pre-push"
    echo "  pre-push hook installed (quality gate + optional delta lint)"
else
    echo "  Skipped: not a git repository"
fi

echo ""
echo "Recommended verification:"
echo "  cargo test --test api_http_integration"
echo "  npm --prefix ui run build"
echo ""
echo "Quick start:"
echo "  Browser mode:"
echo "    cargo run -- api serve --port 8765"
echo "    open http://127.0.0.1:8765"
echo ""
echo "  Desktop mode:"
echo "    npm --prefix ui run build -- --watch"
echo "    cargo run -- api serve --port 8765"
echo "    cargo tauri dev --config src-tauri/tauri.conf.json"
echo ""
echo "  Optional Tauri CLI install:"
echo "    cargo install tauri-cli"
echo ""
echo "Smoke tests:"
echo "  cargo test                           # default test suite"
echo "  cargo test --all-features            # full test suite"
echo "  cargo clippy --all-features          # lint all code"
echo ""
echo "=== Setup complete ==="
