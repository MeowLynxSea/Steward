# Developer Bootstrap

This project is developed as **Steward**.

## Prerequisites

- Rust toolchain `1.92+`
- Node.js `20+` with `npm`
- `wasm32-wasip2` Rust target
- `wasm-tools`
- Optional for desktop shell work: `cargo install tauri-cli`

## One-command bootstrap

Run:

```bash
./scripts/dev-setup.sh
```

That bootstrap script installs the Rust/WASM prerequisites, installs UI
dependencies, builds the static frontend bundle, and installs git hooks.

## Browser mode

1. Build the frontend bundle:

```bash
npm --prefix ui ci
npm --prefix ui run build
```

2. Start the local API:

```bash
cargo run -- api serve --port 8765
```

3. Open:

```text
http://127.0.0.1:8765
```

The Axum server serves the static bundle from `static/` for secondary browser-mode development.

## Desktop mode

Desktop mode is the primary development path. Tauri IPC is the authoritative UI transport; the local browser server remains optional for browser-mode work.

Use two terminals for normal desktop development:

1. Rebuild the static UI on change:

```bash
npm --prefix ui run build -- --watch
```

2. Launch the Tauri shell:

```bash
cargo tauri dev --config tauri.conf.json
```

## Recommended validation

Run the same checks used during migration work before committing:

```bash
cargo test
npm --prefix ui run build
```
