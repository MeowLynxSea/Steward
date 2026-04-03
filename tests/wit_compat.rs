//! WIT compatibility tests for WASM tools.
//!
//! These tests verify that pre-built WASM components can be compiled and
//! instantiated against the current host linker. If the tool WIT interface
//! changes, these tests catch any breakage in existing tools.
//!
//! Prerequisites: build WASM extensions first with:
//!   ./scripts/build-wasm-extensions.sh
//!
//! The tests are skipped (not failed) when no WASM artifacts are found,
//! so `cargo test` still passes without building extensions first.
//! CI runs the build script before these tests.

use std::path::{Path, PathBuf};

use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

struct TestStoreData {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl TestStoreData {
    fn new() -> Self {
        Self {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        }
    }
}

impl WasiView for TestStoreData {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

struct DiscoveredExtension {
    name: String,
    source_dir: PathBuf,
    crate_name: String,
}

fn find_wasm_artifact(source_dir: &Path, crate_name: &str) -> Option<PathBuf> {
    let artifact_name = crate_name.replace('-', "_");

    for target_triple in &["wasm32-wasip2", "wasm32-wasip1", "wasm32-wasi"] {
        let candidate = source_dir
            .join("target")
            .join(target_triple)
            .join("release")
            .join(format!("{artifact_name}.wasm"));
        if candidate.exists() {
            return Some(candidate);
        }
    }

    if let Ok(shared) = std::env::var("CARGO_TARGET_DIR") {
        for target_triple in &["wasm32-wasip2", "wasm32-wasip1", "wasm32-wasi"] {
            let candidate = Path::new(&shared)
                .join(target_triple)
                .join("release")
                .join(format!("{artifact_name}.wasm"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        let shared = home.join(".cargo/shared-target");
        if shared.exists() {
            for target_triple in &["wasm32-wasip2", "wasm32-wasip1", "wasm32-wasi"] {
                let candidate = shared
                    .join(target_triple)
                    .join("release")
                    .join(format!("{artifact_name}.wasm"));
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

fn discover_extensions() -> Vec<DiscoveredExtension> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registry_dir = repo_root.join("registry/tools");
    let mut extensions = Vec::new();

    if !registry_dir.exists() {
        return extensions;
    }

    for entry in std::fs::read_dir(&registry_dir).expect("failed to read registry dir") {
        let entry = entry.expect("failed to read directory entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let content = std::fs::read_to_string(&path).expect("failed to read manifest");
        let manifest: serde_json::Value =
            serde_json::from_str(&content).expect("failed to parse manifest");

        let name = manifest["name"].as_str().unwrap_or("unknown").to_string();
        let source_dir = manifest["source"]["dir"]
            .as_str()
            .map(|d| repo_root.join(d));
        let crate_name = manifest["source"]["crate_name"]
            .as_str()
            .map(|s| s.to_string());

        if let (Some(source_dir), Some(crate_name)) = (source_dir, crate_name)
            && source_dir.exists()
        {
            extensions.push(DiscoveredExtension {
                name,
                source_dir,
                crate_name,
            });
        }
    }

    extensions
}

fn compile_component(
    engine: &wasmtime::Engine,
    wasm_bytes: &[u8],
) -> Result<wasmtime::component::Component, String> {
    wasmtime::component::Component::new(engine, wasm_bytes)
        .map_err(|e| format!("compilation failed: {e}"))
}

fn stub_shared_host_functions(
    host: &mut wasmtime::component::LinkerInstance<'_, TestStoreData>,
) -> Result<(), String> {
    host.func_new("log", |_ctx, _args, _results| Ok(()))
        .map_err(|e| format!("stub 'log': {e}"))?;

    host.func_new("now-millis", |_ctx, _args, results| {
        results[0] = wasmtime::component::Val::U64(0);
        Ok(())
    })
    .map_err(|e| format!("stub 'now-millis': {e}"))?;

    host.func_new("workspace-read", |_ctx, _args, results| {
        results[0] = wasmtime::component::Val::Option(None);
        Ok(())
    })
    .map_err(|e| format!("stub 'workspace-read': {e}"))?;

    host.func_new("http-request", |_ctx, _args, results| {
        results[0] = wasmtime::component::Val::Result(Err(Some(Box::new(
            wasmtime::component::Val::String("stub".into()),
        ))));
        Ok(())
    })
    .map_err(|e| format!("stub 'http-request': {e}"))?;

    host.func_new("secret-exists", |_ctx, _args, results| {
        results[0] = wasmtime::component::Val::Bool(false);
        Ok(())
    })
    .map_err(|e| format!("stub 'secret-exists': {e}"))?;

    Ok(())
}

fn instantiate_tool_component(
    engine: &wasmtime::Engine,
    component: &wasmtime::component::Component,
) -> Result<(), String> {
    use wasmtime::Store;
    use wasmtime::component::Linker;

    let mut linker: Linker<TestStoreData> = Linker::new(engine);

    wasmtime_wasi::add_to_linker_sync(&mut linker)
        .map_err(|e| format!("WASI linker failed: {e}"))?;

    for interface in &["near:agent/host", "near:agent/host@0.3.0"] {
        let mut root = linker.root();
        if let Ok(mut host) = root.instance(interface) {
            stub_shared_host_functions(&mut host)?;

            host.func_new("tool-invoke", |_ctx, _args, results| {
                results[0] = wasmtime::component::Val::Result(Err(Some(Box::new(
                    wasmtime::component::Val::String("stub".into()),
                ))));
                Ok(())
            })
            .map_err(|e| format!("stub 'tool-invoke': {e}"))?;
        }
    }

    let mut store = Store::new(engine, TestStoreData::new());
    linker
        .instantiate(&mut store, component)
        .map_err(|e| format!("instantiation failed: {e}"))?;

    Ok(())
}

fn create_engine() -> wasmtime::Engine {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    config.wasm_threads(false);
    wasmtime::Engine::new(&config).expect("failed to create wasmtime engine")
}

#[test]
fn wit_compat_tool_components_compile_and_instantiate() {
    let extensions = discover_extensions();
    let engine = create_engine();

    if extensions.is_empty() {
        eprintln!("SKIP: no tool extensions found in registry");
        return;
    }

    let mut found_any = false;
    let mut failures: Vec<String> = Vec::new();

    for ext in &extensions {
        let wasm_path = match find_wasm_artifact(&ext.source_dir, &ext.crate_name) {
            Some(p) => p,
            None => {
                eprintln!(
                    "  SKIP {}: no built WASM artifact (run ./scripts/build-wasm-extensions.sh)",
                    ext.name
                );
                continue;
            }
        };

        found_any = true;
        eprintln!("  TEST {}: {}", ext.name, wasm_path.display());

        let wasm_bytes = std::fs::read(&wasm_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", wasm_path.display()));

        let component = match compile_component(&engine, &wasm_bytes) {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("{}: {e}", ext.name));
                continue;
            }
        };

        if let Err(e) = instantiate_tool_component(&engine, &component) {
            failures.push(format!("{}: {e}", ext.name));
        }
    }

    if !found_any {
        eprintln!("SKIP: no WASM artifacts found (build extensions first)");
        return;
    }

    assert!(
        failures.is_empty(),
        "WIT compatibility failures for tools:\n{}",
        failures.join("\n")
    );
}

#[test]
fn wit_compat_all_registry_extensions_have_source() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registry_dir = repo_root.join("registry/tools");
    let mut missing = Vec::new();

    if !registry_dir.exists() {
        return;
    }

    for entry in std::fs::read_dir(&registry_dir).expect("failed to read registry dir") {
        let entry = entry.expect("failed to read directory entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let content = std::fs::read_to_string(&path).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&content).unwrap();

        let name = manifest["name"].as_str().unwrap_or("unknown");
        let source_dir = manifest["source"]["dir"].as_str();
        let crate_name = manifest["source"]["crate_name"].as_str();

        match (source_dir, crate_name) {
            (Some(d), Some(_)) => {
                if !repo_root.join(d).exists() {
                    missing.push(format!("{name}: source dir '{d}' does not exist"));
                }
            }
            _ => {
                missing.push(format!("{name}: missing source.dir or source.crate_name"));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "Registry entries with missing sources:\n{}",
        missing.join("\n")
    );
}

#[test]
fn wit_files_contain_version_annotation() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = repo_root.join("wit/tool.wit");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read wit/tool.wit: {e}"));

    assert!(
        content.contains("package near:agent@"),
        "wit/tool.wit must contain a versioned package declaration (e.g., 'package near:agent@0.3.0;')"
    );
}

#[test]
fn wit_version_constants_match_wit_files() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let tool_wit = std::fs::read_to_string(repo_root.join("wit/tool.wit"))
        .expect("failed to read wit/tool.wit");
    let expected_tool = format!(
        "package near:agent@{};",
        ironclaw::tools::wasm::WIT_TOOL_VERSION
    );

    assert!(
        tool_wit.contains(&expected_tool),
        "wit/tool.wit version must match WIT_TOOL_VERSION constant ({})",
        ironclaw::tools::wasm::WIT_TOOL_VERSION
    );
}
