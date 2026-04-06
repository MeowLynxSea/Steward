fn main() {
    // Tauri build must be called first
    tauri_build::build();

    // Registry embedding
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = std::path::PathBuf::from(&manifest_dir);
    embed_registry_catalog(&root);
}

fn embed_registry_catalog(root: &std::path::Path) {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    let registry_dir = root.join("registry");
    println!("cargo:rerun-if-changed=registry/_bundles.json");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let out_path = out_dir.join("embedded_catalog.json");

    if !registry_dir.is_dir() {
        fs::write(
            &out_path,
            r#"{"tools":[],"mcp_servers":[],"bundles":{"bundles":{}}}"#,
        )
        .unwrap();
        return;
    }

    let mut tools = Vec::new();
    let mut mcp_servers = Vec::new();

    let tools_dir = registry_dir.join("tools");
    if tools_dir.is_dir() {
        collect_json_files(&tools_dir, &mut tools);
    }

    let mcp_servers_dir = registry_dir.join("mcp-servers");
    if mcp_servers_dir.is_dir() {
        collect_json_files(&mcp_servers_dir, &mut mcp_servers);
    }

    let bundles_path = registry_dir.join("_bundles.json");
    let bundles_raw = if bundles_path.is_file() {
        fs::read_to_string(&bundles_path).unwrap_or_else(|_| r#"{"bundles":{}}"#.to_string())
    } else {
        r#"{"bundles":{}}"#.to_string()
    };

    let catalog = format!(
        r#"{{"tools":[{}],"mcp_servers":[{}],"bundles":{}}}"#,
        tools.join(","),
        mcp_servers.join(","),
        bundles_raw,
    );
    fs::write(&out_path, catalog).unwrap();
}

fn collect_json_files(dir: &std::path::Path, out: &mut Vec<String>) {
    use std::fs;

    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("json")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        println!("cargo:rerun-if-changed={}", entry.path().display());
        if let Ok(content) = fs::read_to_string(entry.path()) {
            out.push(content);
        }
    }
}
