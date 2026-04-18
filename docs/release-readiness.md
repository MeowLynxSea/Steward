# Steward Release Readiness

## Supported Packaging Targets

Steward packaging is organized around the Tauri desktop shell.

Supported host-to-bundle targets:

- macOS host: `app`, `dmg`
- Linux host: `appimage`, `deb`
- Windows host: `msi`, `nsis`

Browser mode remains a supported local runtime path, but it is not a desktop bundle target.

## Required Assets And Metadata

Release builds should verify:

- product name is `Steward`
- bundle identifier is `ai.steward.desktop`
- tray/window title uses `Steward`
- `src-tauri/icons/icon.png`, `src-tauri/icons/icon.icns`, and `src-tauri/icons/icon.ico` are real branded assets, not placeholders

## Build Commands

### Frontend bundle

```bash
npm --prefix ui ci
npm --prefix ui run build
```

### Core backend verification

```bash
cargo test --test api_http_integration
```

### Desktop bundle verification

Install the Tauri CLI if needed:

```bash
cargo install tauri-cli
```

Build commands by host OS:

```bash
# macOS
cargo tauri build --config tauri.conf.json --bundles app,dmg

# Linux
cargo tauri build --config tauri.conf.json --bundles appimage,deb

# Windows
cargo tauri build --config tauri.conf.json --bundles msi,nsis
```

## Release Checklist

1. Run `npm --prefix ui run build`.
2. Run `cargo test --test api_http_integration`.
3. Build the Tauri bundles for the current host OS.
4. Start the packaged or dev desktop shell against the local backend and verify:
   - session creation works
   - run history loads
   - Ask approvals pause execution correctly
   - workspace indexing/search still work
   - folder-drop indexing works in desktop mode
5. Verify the app icon, product name, and bundle identifier are branded as Steward.
6. Confirm docs still match the product:
   - [user-guide.md](./user-guide.md)
   - [LLM_PROVIDERS.md](./LLM_PROVIDERS.md)

## Release Notes Focus

When preparing a release, emphasize:

- session-first agent workflow
- Ask/Yolo supervision model
- local workspace indexing and retrieval
- browser-vs-desktop runtime options

Avoid presenting the product as:

- a predefined workflow runner
- a hosted remote agent service
- a cloud account dependent product
