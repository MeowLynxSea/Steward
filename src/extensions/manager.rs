//! Desktop-first extension manager.
//!
//! Desktop/Tauri IPC remains the primary product surface, while optional
//! WASM channels can be activated as secondary ingress/egress adapters.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::channels::wasm::{ChannelCapabilitiesFile, WasmChannelLoader, WasmChannelRuntime};
use crate::channels::ChannelManager;
use crate::extensions::discovery::OnlineDiscovery;
use crate::extensions::registry::ExtensionRegistry;
use crate::extensions::setup_schema::{SecretFieldInfo, SetupFieldInfo};
use crate::extensions::{
    ActivateResult, AuthResult, ConfigureResult, ExtensionError, ExtensionKind, ExtensionSource,
    InstallResult, InstalledExtension, RegistryEntry, ResultSource, SearchResult, ToolAuthState,
    UpgradeOutcome, UpgradeResult,
};
use crate::hooks::HookRegistry;
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::tools::ToolRegistry;
use crate::tools::mcp::auth::{authorize_mcp_server, is_authenticated};
use crate::tools::mcp::config::{McpServerConfig, McpServersFile};
use crate::tools::mcp::create_client_from_config;
use crate::tools::mcp::session::McpSessionManager;
use crate::tools::mcp::{McpClient, McpProcessManager};
use crate::tools::wasm::{
    CapabilitiesFile, ToolFieldSetupSchema, WasmToolLoader, WasmToolRuntime,
    check_wit_version_compat, discover_tools,
};

/// Setup schema returned to UI callers.
pub struct ExtensionSetupSchema {
    pub secrets: Vec<SecretFieldInfo>,
    pub fields: Vec<SetupFieldInfo>,
}

const ALLOWED_GLOBAL_SETUP_SETTING_PATHS: &[&str] = &[
    "llm_backend",
    "selected_model",
    "ollama_base_url",
    "openai_compatible_base_url",
];

pub struct ExtensionManager {
    registry: ExtensionRegistry,
    discovery: OnlineDiscovery,
    mcp_session_manager: Arc<McpSessionManager>,
    mcp_process_manager: Arc<McpProcessManager>,
    mcp_clients: RwLock<HashMap<String, Arc<McpClient>>>,
    wasm_tool_runtime: Option<Arc<WasmToolRuntime>>,
    wasm_tools_dir: PathBuf,
    wasm_channels_dir: PathBuf,
    channel_manager: RwLock<Option<Arc<ChannelManager>>>,
    wasm_channel_runtime: RwLock<Option<Arc<WasmChannelRuntime>>>,
    active_channel_names: RwLock<HashSet<String>>,
    activation_errors: RwLock<HashMap<String, String>>,
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    tool_registry: Arc<ToolRegistry>,
    hooks: Option<Arc<HookRegistry>>,
    user_id: String,
    store: Option<Arc<dyn crate::db::Database>>,
}

impl ExtensionManager {
    pub fn owner_id(&self) -> &str {
        &self.user_id
    }

    pub async fn active_tool_names(&self) -> HashSet<String> {
        let mut names = HashSet::new();
        if let Ok(extensions) = self.list(None, false, &self.user_id).await {
            for extension in extensions {
                if !extension.active {
                    continue;
                }
                match extension.kind {
                    ExtensionKind::WasmTool => {
                        names.insert(extension.name);
                    }
                    ExtensionKind::McpServer => {
                        names.extend(extension.tools);
                    }
                    ExtensionKind::WasmChannel => {}
                }
            }
        }
        names
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mcp_session_manager: Arc<McpSessionManager>,
        mcp_process_manager: Arc<McpProcessManager>,
        secrets: Arc<dyn SecretsStore + Send + Sync>,
        tool_registry: Arc<ToolRegistry>,
        hooks: Option<Arc<HookRegistry>>,
        wasm_tool_runtime: Option<Arc<WasmToolRuntime>>,
        wasm_tools_dir: PathBuf,
        wasm_channels_dir: PathBuf,
        _tunnel_url: Option<String>,
        user_id: String,
        store: Option<Arc<dyn crate::db::Database>>,
        catalog_entries: Vec<RegistryEntry>,
    ) -> Self {
        let registry = if catalog_entries.is_empty() {
            ExtensionRegistry::new()
        } else {
            ExtensionRegistry::new_with_catalog(catalog_entries)
        };

        Self {
            registry,
            discovery: OnlineDiscovery::new(),
            mcp_session_manager,
            mcp_process_manager,
            mcp_clients: RwLock::new(HashMap::new()),
            wasm_tool_runtime,
            wasm_tools_dir,
            wasm_channels_dir,
            channel_manager: RwLock::new(None),
            wasm_channel_runtime: RwLock::new(None),
            active_channel_names: RwLock::new(HashSet::new()),
            activation_errors: RwLock::new(HashMap::new()),
            secrets,
            tool_registry,
            hooks,
            user_id,
            store,
        }
    }

    pub async fn inject_registry_entry(&self, entry: RegistryEntry) {
        self.registry.cache_discovered(vec![entry]).await;
    }

    pub(crate) async fn inject_mcp_client(&self, name: String, client: Arc<McpClient>) {
        self.mcp_clients.write().await.insert(name, client);
    }

    pub(crate) async fn notification_target_for_channel(&self, _name: &str) -> Option<String> {
        Some(self.user_id.clone())
    }

    pub fn secrets(&self) -> &Arc<dyn SecretsStore + Send + Sync> {
        &self.secrets
    }

    pub async fn set_active_channels(&self, names: Vec<String>) {
        let deduped: HashSet<String> = names.into_iter().collect();
        *self.active_channel_names.write().await = deduped.clone();

        let Some(store) = &self.store else {
            return;
        };

        let mut sorted: Vec<String> = deduped.into_iter().collect();
        sorted.sort();
        if let Err(error) = store
            .set_setting(
                &self.user_id,
                "extensions.active_wasm_channels",
                &serde_json::json!(sorted),
            )
            .await
        {
            tracing::warn!(%error, "Failed to persist active wasm channels");
        }
    }

    pub async fn load_persisted_active_channels(&self, user_id: &str) -> Vec<String> {
        let Some(store) = &self.store else {
            return self.active_channel_names.read().await.iter().cloned().collect();
        };

        match store
            .get_setting(user_id, "extensions.active_wasm_channels")
            .await
        {
            Ok(Some(value)) => serde_json::from_value::<Vec<String>>(value).unwrap_or_default(),
            Ok(None) | Err(_) => self.active_channel_names.read().await.iter().cloned().collect(),
        }
    }

    pub async fn set_channel_runtime(
        &self,
        channel_manager: Arc<ChannelManager>,
        wasm_channel_runtime: Arc<WasmChannelRuntime>,
    ) {
        *self.channel_manager.write().await = Some(channel_manager);
        *self.wasm_channel_runtime.write().await = Some(wasm_channel_runtime);
    }

    pub async fn search(
        &self,
        query: &str,
        discover: bool,
    ) -> Result<Vec<SearchResult>, ExtensionError> {
        let mut results = self.registry.search(query).await;
        if discover && results.is_empty() {
            let discovered = self.discovery.discover(query).await;
            if !discovered.is_empty() {
                self.registry.cache_discovered(discovered.clone()).await;
                for entry in discovered {
                    results.push(SearchResult {
                        entry,
                        source: ResultSource::Discovered,
                        validated: true,
                    });
                }
            }
        }
        Ok(results)
    }

    pub async fn install(
        &self,
        name: &str,
        url: Option<&str>,
        kind_hint: Option<ExtensionKind>,
        user_id: &str,
    ) -> Result<InstallResult, ExtensionError> {
        Self::validate_extension_name(name)?;

        if let Some(entry) = self.registry.get_with_kind(name, kind_hint).await {
            return self.install_from_entry(&entry, user_id).await;
        }

        if let Some(url) = url {
            return match kind_hint.unwrap_or_else(|| infer_kind_from_url(url)) {
                ExtensionKind::McpServer => self.install_mcp_from_url(name, url, user_id).await,
                ExtensionKind::WasmTool => {
                    self.install_wasm_tool_from_url_with_caps(name, url, None)
                        .await
                }
                ExtensionKind::WasmChannel => {
                    self.install_wasm_channel_from_url_with_caps(name, url, None)
                        .await
                }
            };
        }

        Err(ExtensionError::NotFound(format!(
            "'{}' not found in registry. Try discover:true or provide a URL.",
            name
        )))
    }

    pub async fn auth(&self, name: &str, user_id: &str) -> Result<AuthResult, ExtensionError> {
        match self.determine_installed_kind(name, user_id).await? {
            ExtensionKind::McpServer => self.auth_mcp(name, user_id).await,
            ExtensionKind::WasmTool => self.auth_wasm_tool(name, user_id).await,
            ExtensionKind::WasmChannel => self.auth_wasm_channel(name, user_id).await,
        }
    }

    pub async fn activate(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<ActivateResult, ExtensionError> {
        Self::validate_extension_name(name)?;
        match self.determine_installed_kind(name, user_id).await? {
            ExtensionKind::McpServer => self.activate_mcp(name, user_id).await,
            ExtensionKind::WasmTool => self.activate_wasm_tool(name, user_id).await,
            ExtensionKind::WasmChannel => self.activate_wasm_channel(name, user_id).await,
        }
    }

    pub async fn list(
        &self,
        kind_filter: Option<ExtensionKind>,
        include_available: bool,
        user_id: &str,
    ) -> Result<Vec<InstalledExtension>, ExtensionError> {
        let mut extensions = Vec::new();

        if kind_filter.is_none() || kind_filter == Some(ExtensionKind::McpServer) {
            if let Ok(servers) = self.load_mcp_servers(user_id).await {
                for server in servers.servers {
                    let authenticated = is_authenticated(&server, &self.secrets, user_id).await;
                    let active = self.mcp_clients.read().await.contains_key(&server.name);
                    let tools = if active {
                        self.tool_registry
                            .list()
                            .await
                            .into_iter()
                            .filter(|tool| tool.starts_with(&format!("{}_", server.name)))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let registry_entry = self
                        .registry
                        .get_with_kind(&server.name, Some(ExtensionKind::McpServer))
                        .await;
                    extensions.push(InstalledExtension {
                        name: server.name.clone(),
                        kind: ExtensionKind::McpServer,
                        display_name: registry_entry
                            .as_ref()
                            .map(|entry| entry.display_name.clone()),
                        description: server.description.clone(),
                        url: Some(server.url.clone()),
                        authenticated,
                        active,
                        tools,
                        needs_setup: false,
                        has_auth: server.requires_auth(),
                        installed: true,
                        activation_error: None,
                        version: registry_entry.and_then(|entry| entry.version.clone()),
                    });
                }
            }
        }

        if kind_filter.is_none() || kind_filter == Some(ExtensionKind::WasmTool) {
            if let Ok(tools) = discover_tools(&self.wasm_tools_dir).await {
                for (name, discovered) in tools {
                    let active = self.tool_registry.has(&name).await;
                    let registry_entry = self
                        .registry
                        .get_with_kind(&name, Some(ExtensionKind::WasmTool))
                        .await;
                    let auth_state = self.check_tool_auth_status(&name, user_id).await;
                    let version = if let Some(cap_path) = &discovered.capabilities_path {
                        tokio::fs::read(cap_path)
                            .await
                            .ok()
                            .and_then(|bytes| CapabilitiesFile::from_bytes(&bytes).ok())
                            .and_then(|cap| cap.version)
                    } else {
                        None
                    }
                    .or_else(|| {
                        registry_entry
                            .as_ref()
                            .and_then(|entry| entry.version.clone())
                    });
                    extensions.push(InstalledExtension {
                        name: name.clone(),
                        kind: ExtensionKind::WasmTool,
                        display_name: registry_entry
                            .as_ref()
                            .map(|entry| entry.display_name.clone()),
                        description: registry_entry
                            .as_ref()
                            .map(|entry| entry.description.clone()),
                        url: None,
                        authenticated: auth_state == ToolAuthState::Ready,
                        active,
                        tools: if active { vec![name] } else { Vec::new() },
                        needs_setup: auth_state == ToolAuthState::NeedsSetup,
                        has_auth: auth_state != ToolAuthState::NoAuth,
                        installed: true,
                        activation_error: None,
                        version,
                    });
                }
            }
        }

        if kind_filter.is_none() || kind_filter == Some(ExtensionKind::WasmChannel) {
            if let Ok(channels) = crate::channels::wasm::discover_channels(&self.wasm_channels_dir).await {
                let active_channels = self.active_channel_names.read().await.clone();
                let activation_errors = self.activation_errors.read().await.clone();
                for (name, discovered) in channels {
                    let registry_entry = self
                        .registry
                        .get_with_kind(&name, Some(ExtensionKind::WasmChannel))
                        .await;
                    let auth_state = self.check_channel_auth_status(&name, user_id).await;
                    let version = if let Some(cap_path) = &discovered.capabilities_path {
                        tokio::fs::read(cap_path)
                            .await
                            .ok()
                            .and_then(|bytes| ChannelCapabilitiesFile::from_bytes(&bytes).ok())
                            .and_then(|cap| cap.version)
                    } else {
                        None
                    }
                    .or_else(|| registry_entry.as_ref().and_then(|entry| entry.version.clone()));
                    let description = if let Some(cap_path) = &discovered.capabilities_path {
                        tokio::fs::read(cap_path)
                            .await
                            .ok()
                            .and_then(|bytes| ChannelCapabilitiesFile::from_bytes(&bytes).ok())
                            .and_then(|cap| cap.description)
                    } else {
                        registry_entry.as_ref().map(|entry| entry.description.clone())
                    };
                    extensions.push(InstalledExtension {
                        name: name.clone(),
                        kind: ExtensionKind::WasmChannel,
                        display_name: registry_entry
                            .as_ref()
                            .map(|entry| entry.display_name.clone()),
                        description,
                        url: None,
                        authenticated: auth_state == ToolAuthState::Ready,
                        active: active_channels.contains(&name),
                        tools: Vec::new(),
                        needs_setup: auth_state == ToolAuthState::NeedsSetup,
                        has_auth: auth_state != ToolAuthState::NoAuth,
                        installed: true,
                        activation_error: activation_errors.get(&name).cloned(),
                        version,
                    });
                }
            }
        }

        if include_available {
            let installed: HashSet<(String, ExtensionKind)> = extensions
                .iter()
                .map(|entry| (entry.name.clone(), entry.kind))
                .collect();
            for entry in self.registry.all_entries().await {
                if let Some(filter) = kind_filter
                    && entry.kind != filter
                {
                    continue;
                }
                if installed.contains(&(entry.name.clone(), entry.kind)) {
                    continue;
                }
                extensions.push(InstalledExtension {
                    name: entry.name,
                    kind: entry.kind,
                    display_name: Some(entry.display_name),
                    description: Some(entry.description),
                    url: None,
                    authenticated: false,
                    active: false,
                    tools: Vec::new(),
                    needs_setup: false,
                    has_auth: false,
                    installed: false,
                    activation_error: None,
                    version: entry.version,
                });
            }
        }

        Ok(extensions)
    }

    pub async fn remove(&self, name: &str, user_id: &str) -> Result<String, ExtensionError> {
        Self::validate_extension_name(name)?;
        match self.determine_installed_kind(name, user_id).await? {
            ExtensionKind::McpServer => {
                let tool_names: Vec<String> = self
                    .tool_registry
                    .list()
                    .await
                    .into_iter()
                    .filter(|tool| tool.starts_with(&format!("{}_", name)))
                    .collect();
                for tool in &tool_names {
                    self.tool_registry.unregister(tool).await;
                }
                self.mcp_clients.write().await.remove(name);
                self.remove_mcp_server(name, user_id).await?;
                if let Ok(server) = self.get_mcp_server(name, user_id).await {
                    let _ = self
                        .secrets
                        .delete(user_id, &server.token_secret_name())
                        .await;
                    let _ = self
                        .secrets
                        .delete(user_id, &server.refresh_token_secret_name())
                        .await;
                }
                Ok(format!(
                    "Removed MCP server '{}' and {} tool(s)",
                    name,
                    tool_names.len()
                ))
            }
            ExtensionKind::WasmTool => {
                self.tool_registry.unregister(name).await;
                if let Some(runtime) = &self.wasm_tool_runtime {
                    runtime.remove(name).await;
                }
                self.unregister_hook_prefix(&format!("plugin.tool:{}::", name))
                    .await;
                self.unregister_hook_prefix(&format!("plugin.dev_tool:{}::", name))
                    .await;

                let cap = self.load_tool_capabilities(name).await;
                if let Some(cap) = &cap {
                    for secret_name in Self::tool_secret_names(cap) {
                        let _ = self.secrets.delete(user_id, &secret_name).await;
                    }
                    if let Some(auth) = &cap.auth {
                        let _ = self.secrets.delete(user_id, &auth.secret_name).await;
                        let _ = self
                            .secrets
                            .delete(user_id, &format!("{}_refresh_token", auth.secret_name))
                            .await;
                        let _ = self
                            .secrets
                            .delete(user_id, &format!("{}_scopes", auth.secret_name))
                            .await;
                    }
                }

                let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));
                let cap_path = self
                    .wasm_tools_dir
                    .join(format!("{}.capabilities.json", name));
                if wasm_path.exists() {
                    tokio::fs::remove_file(&wasm_path)
                        .await
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;
                }
                if cap_path.exists() {
                    let _ = tokio::fs::remove_file(&cap_path).await;
                }
                Ok(format!("Removed WASM tool '{}'", name))
            }
            ExtensionKind::WasmChannel => {
                self.activation_errors.write().await.remove(name);
                self.active_channel_names.write().await.remove(name);
                let persisted = self
                    .active_channel_names
                    .read()
                    .await
                    .iter()
                    .cloned()
                    .collect();
                self.set_active_channels(persisted).await;

                let wasm_path = self.wasm_channels_dir.join(format!("{}.wasm", name));
                let cap_path = self
                    .wasm_channels_dir
                    .join(format!("{}.capabilities.json", name));
                if wasm_path.exists() {
                    tokio::fs::remove_file(&wasm_path)
                        .await
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;
                }
                if cap_path.exists() {
                    let _ = tokio::fs::remove_file(&cap_path).await;
                }
                Ok(format!("Removed WASM channel '{}'", name))
            }
        }
    }

    pub async fn upgrade(
        &self,
        name: Option<&str>,
        _user_id: &str,
    ) -> Result<UpgradeResult, ExtensionError> {
        let mut candidates = Vec::new();
        if let Some(name) = name {
            candidates.push(name.to_string());
        } else if let Ok(tools) = discover_tools(&self.wasm_tools_dir).await {
            candidates.extend(tools.keys().cloned());
            if let Ok(channels) = crate::channels::wasm::discover_channels(&self.wasm_channels_dir).await {
                candidates.extend(channels.keys().cloned());
            }
        }

        let mut results = Vec::new();
        for tool_name in candidates {
            let outcome = self.upgrade_one_tool(&tool_name).await;
            results.push(outcome);
        }
        let upgraded = results
            .iter()
            .filter(|item| item.status == "upgraded")
            .count();
        let up_to_date = results
            .iter()
            .filter(|item| item.status == "already_up_to_date")
            .count();
        let failed = results
            .iter()
            .filter(|item| item.status == "failed")
            .count();
        Ok(UpgradeResult {
            message: format!(
                "{} extension(s) checked: {} upgraded, {} already up to date, {} failed",
                results.len(),
                upgraded,
                up_to_date,
                failed
            ),
            results,
        })
    }

    pub async fn extension_info(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<serde_json::Value, ExtensionError> {
        match self.determine_installed_kind(name, user_id).await? {
            ExtensionKind::McpServer => Ok(serde_json::json!({
                "name": name,
                "kind": "mcp_server",
                "connected": self.mcp_clients.read().await.contains_key(name),
            })),
            ExtensionKind::WasmTool => {
                let cap_path = self
                    .wasm_tools_dir
                    .join(format!("{}.capabilities.json", name));
                let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));
                let mut value = serde_json::json!({
                    "name": name,
                    "kind": "wasm_tool",
                    "installed": wasm_path.exists(),
                    "host_wit_version": crate::tools::wasm::WIT_TOOL_VERSION,
                });
                if cap_path.exists()
                    && let Ok(bytes) = tokio::fs::read(&cap_path).await
                    && let Ok(cap) = CapabilitiesFile::from_bytes(&bytes)
                {
                    value["version"] =
                        serde_json::json!(cap.version.unwrap_or_else(|| "unknown".to_string()));
                    value["wit_version"] =
                        serde_json::json!(cap.wit_version.unwrap_or_else(|| "unknown".to_string()));
                }
                Ok(value)
            }
            ExtensionKind::WasmChannel => {
                let cap_path = self
                    .wasm_channels_dir
                    .join(format!("{}.capabilities.json", name));
                let wasm_path = self.wasm_channels_dir.join(format!("{}.wasm", name));
                let mut value = serde_json::json!({
                    "name": name,
                    "kind": "wasm_channel",
                    "installed": wasm_path.exists(),
                    "active": self.active_channel_names.read().await.contains(name),
                });
                if cap_path.exists()
                    && let Ok(bytes) = tokio::fs::read(&cap_path).await
                    && let Ok(cap) = ChannelCapabilitiesFile::from_bytes(&bytes)
                {
                    value["version"] =
                        serde_json::json!(cap.version.unwrap_or_else(|| "unknown".to_string()));
                    value["description"] = serde_json::json!(cap.description);
                    value["allowed_paths"] = serde_json::json!(cap.capabilities
                        .channel
                        .as_ref()
                        .map(|c| c.allowed_paths.clone())
                        .unwrap_or_default());
                }
                Ok(value)
            }
        }
    }

    pub async fn get_setup_schema(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<ExtensionSetupSchema, ExtensionError> {
        match self.determine_installed_kind(name, user_id).await? {
            ExtensionKind::McpServer => Ok(ExtensionSetupSchema {
                secrets: Vec::new(),
                fields: Vec::new(),
            }),
            ExtensionKind::WasmTool => {
                let Some(cap_file) = self.load_tool_capabilities(name).await else {
                    return Ok(ExtensionSetupSchema {
                        secrets: Vec::new(),
                        fields: Vec::new(),
                    });
                };
                let mut secrets = Vec::new();
                let mut fields = Vec::new();
                let saved_fields = self.load_tool_setup_fields(name).await.unwrap_or_default();
                if let Some(setup) = &cap_file.setup {
                    for secret in &setup.required_secrets {
                        let provided = self
                            .secrets
                            .exists(user_id, &secret.name)
                            .await
                            .unwrap_or(false);
                        secrets.push(SecretFieldInfo {
                            name: secret.name.clone(),
                            prompt: secret.prompt.clone(),
                            optional: secret.optional,
                            provided,
                            auto_generate: false,
                        });
                    }
                    for field in &setup.required_fields {
                        fields.push(SetupFieldInfo {
                            name: field.name.clone(),
                            prompt: field.prompt.clone(),
                            optional: field.optional,
                            provided: self
                                .is_tool_setup_field_provided(name, field, &saved_fields)
                                .await,
                            input_type: field.input_type,
                        });
                    }
                }
                Ok(ExtensionSetupSchema { secrets, fields })
            }
            ExtensionKind::WasmChannel => {
                let Some(cap_file) = self.load_channel_capabilities(name).await else {
                    return Ok(ExtensionSetupSchema {
                        secrets: Vec::new(),
                        fields: Vec::new(),
                    });
                };
                let secrets = futures::future::join_all(
                    cap_file
                        .setup
                        .required_secrets
                        .iter()
                        .map(|secret| async move {
                            let provided = self
                                .secrets
                                .exists(user_id, &secret.name)
                                .await
                                .unwrap_or(false);
                            SecretFieldInfo {
                                name: secret.name.clone(),
                                prompt: secret.prompt.clone(),
                                optional: secret.optional,
                                provided,
                                auto_generate: secret.auto_generate.is_some(),
                            }
                        }),
                )
                .await;
                Ok(ExtensionSetupSchema {
                    secrets,
                    fields: Vec::new(),
                })
            }
        }
    }

    pub async fn configure(
        &self,
        name: &str,
        secrets: &HashMap<String, String>,
        fields: &HashMap<String, String>,
        user_id: &str,
    ) -> Result<ConfigureResult, ExtensionError> {
        let kind = self.determine_installed_kind(name, user_id).await?;
        let mut restart_required = false;

        match kind {
            ExtensionKind::McpServer => {
                let server = self.get_mcp_server(name, user_id).await?;
                if let Some(token) = secrets.values().find(|value| !value.trim().is_empty()) {
                    let params = CreateSecretParams::new(server.token_secret_name(), token.trim())
                        .with_provider(name.to_string());
                    self.secrets
                        .create(user_id, params)
                        .await
                        .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
                }
            }
            ExtensionKind::WasmTool => {
                let cap = self.load_tool_capabilities(name).await.ok_or_else(|| {
                    ExtensionError::Other(format!("Capabilities file not found for '{}'", name))
                })?;

                let mut allowed_secrets = HashSet::new();
                let mut setup_fields = Vec::<ToolFieldSetupSchema>::new();
                if let Some(setup) = &cap.setup {
                    allowed_secrets
                        .extend(setup.required_secrets.iter().map(|item| item.name.clone()));
                    setup_fields = setup.required_fields.clone();
                }
                if let Some(auth) = &cap.auth {
                    allowed_secrets.insert(auth.secret_name.clone());
                }

                for (secret_name, secret_value) in secrets {
                    if !allowed_secrets.contains(secret_name) || secret_value.trim().is_empty() {
                        continue;
                    }
                    let params = CreateSecretParams::new(secret_name, secret_value.trim())
                        .with_provider(name.to_string());
                    self.secrets
                        .create(user_id, params)
                        .await
                        .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
                }

                let mut stored_fields = self.load_tool_setup_fields(name).await.unwrap_or_default();
                for field in &setup_fields {
                    if let Some(value) = fields.get(&field.name) {
                        let trimmed = value.trim();
                        if trimmed.is_empty() {
                            stored_fields.remove(&field.name);
                            if let Some(setting_path) = &field.setting_path
                                && let Some(store) = self.store.as_ref()
                            {
                                let _ = store.delete_setting(&self.user_id, setting_path).await;
                            }
                            continue;
                        }

                        stored_fields.insert(field.name.clone(), trimmed.to_string());
                        restart_required |= field.restart_required;

                        if let Some(setting_path) = &field.setting_path {
                            Self::validate_setup_setting_path(name, setting_path)?;
                            let store = self.store.as_ref().ok_or_else(|| {
                                ExtensionError::Other(
                                    "Settings store unavailable for setup field persistence"
                                        .to_string(),
                                )
                            })?;
                            store
                                .set_setting(
                                    &self.user_id,
                                    setting_path,
                                    &serde_json::Value::String(trimmed.to_string()),
                                )
                                .await
                                .map_err(|e| ExtensionError::Other(e.to_string()))?;
                        }
                    }
                }

                if !setup_fields.is_empty() {
                    self.save_tool_setup_fields(name, &stored_fields).await?;
                }
            }
            ExtensionKind::WasmChannel => {
                let cap = self.load_channel_capabilities(name).await.ok_or_else(|| {
                    ExtensionError::Other(format!("Capabilities file not found for '{}'", name))
                })?;
                let allowed_secrets: HashSet<String> = cap
                    .setup
                    .required_secrets
                    .iter()
                    .map(|item| item.name.clone())
                    .collect();

                for secret in &cap.setup.required_secrets {
                    if secret.optional {
                        continue;
                    }
                    if let Some(value) = secrets.get(&secret.name)
                        && !value.trim().is_empty()
                    {
                        let params = CreateSecretParams::new(&secret.name, value.trim())
                            .with_provider(name.to_string());
                        self.secrets
                            .create(user_id, params)
                            .await
                            .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
                    }
                }
                for (secret_name, secret_value) in secrets {
                    if !allowed_secrets.contains(secret_name) || secret_value.trim().is_empty() {
                        continue;
                    }
                    let params = CreateSecretParams::new(secret_name, secret_value.trim())
                        .with_provider(name.to_string());
                    self.secrets
                        .create(user_id, params)
                        .await
                        .map_err(|e| ExtensionError::AuthFailed(e.to_string()))?;
                }
            }
        }

        let activate_result = self.activate(name, user_id).await;
        match activate_result {
            Ok(result) => Ok(ConfigureResult {
                message: format!(
                    "Configuration saved and '{}' activated. {}",
                    name, result.message
                ),
                activated: true,
                restart_required,
                auth_url: None,
                verification: None,
            }),
            Err(error) => Ok(ConfigureResult {
                message: format!(
                    "Configuration saved for '{}'. Activation failed: {}",
                    name, error
                ),
                activated: false,
                restart_required,
                auth_url: None,
                verification: None,
            }),
        }
    }

    pub async fn configure_token(
        &self,
        name: &str,
        token: &str,
        user_id: &str,
    ) -> Result<ConfigureResult, ExtensionError> {
        let kind = self.determine_installed_kind(name, user_id).await?;
        let secret_name = match kind {
            ExtensionKind::McpServer => self
                .get_mcp_server(name, user_id)
                .await?
                .token_secret_name(),
            ExtensionKind::WasmTool => {
                let cap = self.load_tool_capabilities(name).await.ok_or_else(|| {
                    ExtensionError::Other(format!("Capabilities not found for '{}'", name))
                })?;
                if let Some(auth) = &cap.auth {
                    auth.secret_name.clone()
                } else {
                    cap.setup
                        .as_ref()
                        .and_then(|setup| setup.required_secrets.first())
                        .map(|secret| secret.name.clone())
                        .ok_or_else(|| {
                            ExtensionError::Other(format!(
                                "Tool '{}' has no auth or setup secrets",
                                name
                            ))
                        })?
                }
            }
            ExtensionKind::WasmChannel => {
                let cap = self.load_channel_capabilities(name).await.ok_or_else(|| {
                    ExtensionError::Other(format!("Capabilities not found for '{}'", name))
                })?;
                cap.setup
                    .required_secrets
                    .first()
                    .map(|secret| secret.name.clone())
                    .ok_or_else(|| {
                        ExtensionError::Other(format!(
                            "Channel '{}' has no setup secrets",
                            name
                        ))
                    })?
            }
        };

        let mut payload = HashMap::new();
        payload.insert(secret_name, token.to_string());
        self.configure(name, &payload, &HashMap::new(), user_id)
            .await
    }

    async fn install_from_entry(
        &self,
        entry: &RegistryEntry,
        user_id: &str,
    ) -> Result<InstallResult, ExtensionError> {
        match &entry.source {
            ExtensionSource::McpUrl { url } => {
                self.install_mcp_from_url(&entry.name, url, user_id).await
            }
            ExtensionSource::WasmDownload {
                wasm_url,
                capabilities_url,
            } => match entry.kind {
                ExtensionKind::WasmTool => {
                    self.install_wasm_tool_from_url_with_caps(
                        &entry.name,
                        wasm_url,
                        capabilities_url.as_deref(),
                    )
                    .await
                }
                ExtensionKind::WasmChannel => {
                    self.install_wasm_channel_from_url_with_caps(
                        &entry.name,
                        wasm_url,
                        capabilities_url.as_deref(),
                    )
                    .await
                }
                ExtensionKind::McpServer => unreachable!(),
            },
            ExtensionSource::Discovered { url } => {
                self.install_mcp_from_url(&entry.name, url, user_id).await
            }
            ExtensionSource::WasmBuildable { .. } => Err(ExtensionError::InstallFailed(format!(
                "Buildable extension '{}' is not supported in the stripped runtime",
                entry.name
            ))),
        }
    }

    async fn install_mcp_from_url(
        &self,
        name: &str,
        url: &str,
        user_id: &str,
    ) -> Result<InstallResult, ExtensionError> {
        let config = McpServerConfig::new(name, url);
        if let Some(store) = &self.store {
            crate::tools::mcp::config::add_mcp_server_db(store.as_ref(), user_id, config).await
        } else {
            crate::tools::mcp::config::add_mcp_server(config).await
        }
        .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;

        Ok(InstallResult {
            name: name.to_string(),
            kind: ExtensionKind::McpServer,
            message: format!("Installed MCP server '{}'", name),
        })
    }

    async fn install_wasm_tool_from_url_with_caps(
        &self,
        name: &str,
        wasm_url: &str,
        capabilities_url: Option<&str>,
    ) -> Result<InstallResult, ExtensionError> {
        let client = reqwest::Client::new();
        let wasm_bytes = client
            .get(wasm_url)
            .send()
            .await
            .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?;

        tokio::fs::create_dir_all(&self.wasm_tools_dir)
            .await
            .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;

        let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));
        tokio::fs::write(&wasm_path, &wasm_bytes)
            .await
            .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;

        if let Some(cap_url) = capabilities_url {
            let cap_bytes = client
                .get(cap_url)
                .send()
                .await
                .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?
                .bytes()
                .await
                .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?;
            let cap_path = self
                .wasm_tools_dir
                .join(format!("{}.capabilities.json", name));
            tokio::fs::write(&cap_path, &cap_bytes)
                .await
                .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
        }

        Ok(InstallResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmTool,
            message: format!("Installed WASM tool '{}'", name),
        })
    }

    async fn install_wasm_channel_from_url_with_caps(
        &self,
        name: &str,
        wasm_url: &str,
        capabilities_url: Option<&str>,
    ) -> Result<InstallResult, ExtensionError> {
        let client = reqwest::Client::new();
        let wasm_bytes = client
            .get(wasm_url)
            .send()
            .await
            .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?;

        tokio::fs::create_dir_all(&self.wasm_channels_dir)
            .await
            .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;

        let wasm_path = self.wasm_channels_dir.join(format!("{}.wasm", name));
        tokio::fs::write(&wasm_path, &wasm_bytes)
            .await
            .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;

        if let Some(cap_url) = capabilities_url {
            let cap_bytes = client
                .get(cap_url)
                .send()
                .await
                .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?
                .bytes()
                .await
                .map_err(|e| ExtensionError::DownloadFailed(e.to_string()))?;
            let cap_path = self
                .wasm_channels_dir
                .join(format!("{}.capabilities.json", name));
            tokio::fs::write(&cap_path, &cap_bytes)
                .await
                .map_err(|e| ExtensionError::InstallFailed(e.to_string()))?;
        }

        Ok(InstallResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmChannel,
            message: format!("Installed WASM channel '{}'", name),
        })
    }

    async fn auth_mcp(&self, name: &str, user_id: &str) -> Result<AuthResult, ExtensionError> {
        let server = self.get_mcp_server(name, user_id).await?;
        if is_authenticated(&server, &self.secrets, user_id).await {
            return Ok(AuthResult::authenticated(name, ExtensionKind::McpServer));
        }

        match authorize_mcp_server(&server, &self.secrets, user_id).await {
            Ok(_) => Ok(AuthResult::authenticated(name, ExtensionKind::McpServer)),
            Err(_) => Ok(AuthResult::awaiting_token(
                name,
                ExtensionKind::McpServer,
                format!(
                    "Server '{}' requires credentials. Configure a token in the setup form.",
                    name
                ),
                None,
            )),
        }
    }

    async fn auth_wasm_tool(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<AuthResult, ExtensionError> {
        let Some(cap) = self.load_tool_capabilities(name).await else {
            return Ok(AuthResult::no_auth_required(name, ExtensionKind::WasmTool));
        };

        let Some(auth) = cap.auth else {
            return Ok(AuthResult::no_auth_required(name, ExtensionKind::WasmTool));
        };

        if let Some(env_var) = &auth.env_var
            && std::env::var(env_var)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .is_some()
        {
            return Ok(AuthResult::authenticated(name, ExtensionKind::WasmTool));
        }

        if self
            .secrets
            .exists(user_id, &auth.secret_name)
            .await
            .unwrap_or(false)
        {
            return Ok(AuthResult::authenticated(name, ExtensionKind::WasmTool));
        }

        let setup = cap
            .setup
            .as_ref()
            .and_then(|setup| setup.required_secrets.first());
        let instructions = auth
            .instructions
            .clone()
            .or_else(|| setup.map(|item| item.prompt.clone()))
            .unwrap_or_else(|| format!("Provide credentials for '{}'.", name));

        Ok(AuthResult::awaiting_token(
            name,
            ExtensionKind::WasmTool,
            instructions,
            auth.setup_url.clone(),
        ))
    }

    async fn auth_wasm_channel(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<AuthResult, ExtensionError> {
        let Some(cap) = self.load_channel_capabilities(name).await else {
            return Ok(AuthResult::no_auth_required(name, ExtensionKind::WasmChannel));
        };

        let required: Vec<_> = cap
            .setup
            .required_secrets
            .iter()
            .filter(|secret| !secret.optional)
            .collect();
        if required.is_empty() {
            return Ok(AuthResult::no_auth_required(name, ExtensionKind::WasmChannel));
        }

        let all_present = futures::future::join_all(
            required
                .iter()
                .map(|secret| self.secrets.exists(user_id, &secret.name)),
        )
        .await
        .into_iter()
        .all(|result| result.unwrap_or(false));
        if all_present {
            return Ok(AuthResult::authenticated(name, ExtensionKind::WasmChannel));
        }

        let instructions = cap
            .setup
            .required_secrets
            .first()
            .map(|secret| secret.prompt.clone())
            .unwrap_or_else(|| format!("Configure '{}' before activation.", name));

        Ok(AuthResult::awaiting_token(
            name,
            ExtensionKind::WasmChannel,
            instructions,
            cap.setup.setup_url.clone(),
        ))
    }

    async fn activate_mcp(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<ActivateResult, ExtensionError> {
        if self.mcp_clients.read().await.contains_key(name) {
            let tools = self
                .tool_registry
                .list()
                .await
                .into_iter()
                .filter(|tool| tool.starts_with(&format!("{}_", name)))
                .collect();
            return Ok(ActivateResult {
                name: name.to_string(),
                kind: ExtensionKind::McpServer,
                tools_loaded: tools,
                message: format!("MCP server '{}' already active", name),
            });
        }

        let server = self.get_mcp_server(name, user_id).await?;
        let client = create_client_from_config(
            server.clone(),
            &self.mcp_session_manager,
            &self.mcp_process_manager,
            Some(Arc::clone(&self.secrets)),
            user_id,
        )
        .await
        .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        let mcp_tools = client.list_tools().await.map_err(|e| {
            let msg = e.to_string();
            if msg.contains("401")
                || msg.to_ascii_lowercase().contains("authorization")
                || msg.to_ascii_lowercase().contains("authenticate")
            {
                ExtensionError::AuthRequired
            } else {
                ExtensionError::ActivationFailed(msg)
            }
        })?;

        let tool_names: Vec<String> = mcp_tools
            .iter()
            .map(|tool| format!("{}_{}", name, tool.name))
            .collect();
        for tool in client
            .create_tools()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?
        {
            self.tool_registry.register(tool).await;
        }

        self.mcp_clients
            .write()
            .await
            .insert(name.to_string(), Arc::new(client));

        Ok(ActivateResult {
            name: name.to_string(),
            kind: ExtensionKind::McpServer,
            tools_loaded: tool_names,
            message: format!("Connected to '{}' and loaded tools", name),
        })
    }

    async fn activate_wasm_tool(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<ActivateResult, ExtensionError> {
        if self.tool_registry.has(name).await {
            return Ok(ActivateResult {
                name: name.to_string(),
                kind: ExtensionKind::WasmTool,
                tools_loaded: vec![name.to_string()],
                message: format!("WASM tool '{}' already active", name),
            });
        }

        if self.check_tool_auth_status(name, user_id).await == ToolAuthState::NeedsSetup {
            return Err(ExtensionError::ActivationFailed(format!(
                "Tool '{}' requires configuration before activation",
                name
            )));
        }

        let runtime = self.wasm_tool_runtime.as_ref().ok_or_else(|| {
            ExtensionError::ActivationFailed("WASM runtime not available".to_string())
        })?;

        let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));
        if !wasm_path.exists() {
            return Err(ExtensionError::NotInstalled(format!(
                "WASM tool '{}' not found",
                name
            )));
        }
        let cap_path = self
            .wasm_tools_dir
            .join(format!("{}.capabilities.json", name));
        let cap_path_option = cap_path.exists().then_some(cap_path.as_path());

        WasmToolLoader::new(Arc::clone(runtime), Arc::clone(&self.tool_registry))
            .with_secrets_store(Arc::clone(&self.secrets))
            .load_from_files(name, &wasm_path, cap_path_option)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        if let Some(hooks) = &self.hooks
            && let Some(cap_path) = cap_path_option
        {
            let source = format!("plugin.tool:{}", name);
            let _ = crate::hooks::bootstrap::register_plugin_bundle_from_capabilities_file(
                hooks, &source, cap_path,
            )
            .await;
        }

        Ok(ActivateResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmTool,
            tools_loaded: vec![name.to_string()],
            message: format!("Loaded WASM tool '{}'", name),
        })
    }

    async fn activate_wasm_channel(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<ActivateResult, ExtensionError> {
        if self.active_channel_names.read().await.contains(name) {
            return Ok(ActivateResult {
                name: name.to_string(),
                kind: ExtensionKind::WasmChannel,
                tools_loaded: Vec::new(),
                message: format!("WASM channel '{}' already active", name),
            });
        }

        if self.check_channel_auth_status(name, user_id).await == ToolAuthState::NeedsSetup {
            return Err(ExtensionError::ActivationFailed(format!(
                "Channel '{}' requires configuration before activation",
                name
            )));
        }

        let runtime = self
            .wasm_channel_runtime
            .read()
            .await
            .clone()
            .ok_or_else(|| {
                ExtensionError::ActivationFailed(
                    "WASM channel runtime not available".to_string(),
                )
            })?;
        let channel_manager = self.channel_manager.read().await.clone().ok_or_else(|| {
            ExtensionError::ActivationFailed("Channel manager not available".to_string())
        })?;

        let wasm_path = self.wasm_channels_dir.join(format!("{}.wasm", name));
        if !wasm_path.exists() {
            return Err(ExtensionError::NotInstalled(format!(
                "WASM channel '{}' not found",
                name
            )));
        }
        let cap_path = self
            .wasm_channels_dir
            .join(format!("{}.capabilities.json", name));
        let cap_path_option = cap_path.exists().then_some(cap_path.as_path());

        let loader = WasmChannelLoader::new(runtime, Arc::new(crate::pairing::PairingStore::new()), None, self.user_id.clone())
            .with_secrets_store(Arc::clone(&self.secrets));
        let loaded = loader
            .load_from_files(name, &wasm_path, cap_path_option)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        channel_manager
            .hot_add(Box::new(loaded.channel))
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        self.activation_errors.write().await.remove(name);
        let mut active = self.active_channel_names.read().await.iter().cloned().collect::<Vec<_>>();
        if !active.iter().any(|existing| existing == name) {
            active.push(name.to_string());
        }
        self.set_active_channels(active).await;

        Ok(ActivateResult {
            name: name.to_string(),
            kind: ExtensionKind::WasmChannel,
            tools_loaded: Vec::new(),
            message: format!("Loaded WASM channel '{}'", name),
        })
    }

    async fn determine_installed_kind(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<ExtensionKind, ExtensionError> {
        if self.get_mcp_server(name, user_id).await.is_ok() {
            return Ok(ExtensionKind::McpServer);
        }
        if self.wasm_tools_dir.join(format!("{}.wasm", name)).exists() {
            return Ok(ExtensionKind::WasmTool);
        }
        if self.wasm_channels_dir.join(format!("{}.wasm", name)).exists() {
            return Ok(ExtensionKind::WasmChannel);
        }
        Err(ExtensionError::NotInstalled(format!(
            "'{}' is not installed as an MCP server, WASM tool, or WASM channel",
            name
        )))
    }

    fn validate_extension_name(name: &str) -> Result<(), ExtensionError> {
        if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
            return Err(ExtensionError::InstallFailed(format!(
                "Invalid extension name '{}'",
                name
            )));
        }
        Ok(())
    }

    async fn load_mcp_servers(&self, user_id: &str) -> Result<McpServersFile, ExtensionError> {
        if let Some(store) = &self.store {
            crate::tools::mcp::config::load_mcp_servers_from_db(store.as_ref(), user_id)
                .await
                .map_err(|e| ExtensionError::Config(e.to_string()))
        } else {
            crate::tools::mcp::config::load_mcp_servers()
                .await
                .map_err(|e| ExtensionError::Config(e.to_string()))
        }
    }

    async fn get_mcp_server(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<McpServerConfig, ExtensionError> {
        let servers = self.load_mcp_servers(user_id).await?;
        servers
            .get(name)
            .cloned()
            .ok_or_else(|| ExtensionError::NotInstalled(name.to_string()))
    }

    async fn remove_mcp_server(&self, name: &str, user_id: &str) -> Result<(), ExtensionError> {
        if let Some(store) = &self.store {
            crate::tools::mcp::config::remove_mcp_server_db(store.as_ref(), user_id, name)
                .await
                .map_err(|e| ExtensionError::Config(e.to_string()))
        } else {
            crate::tools::mcp::config::remove_mcp_server(name)
                .await
                .map_err(|e| ExtensionError::Config(e.to_string()))
        }
    }

    async fn load_tool_capabilities(&self, name: &str) -> Option<CapabilitiesFile> {
        let cap_path = self
            .wasm_tools_dir
            .join(format!("{}.capabilities.json", name));
        let bytes = tokio::fs::read(cap_path).await.ok()?;
        CapabilitiesFile::from_bytes(&bytes).ok()
    }

    async fn load_channel_capabilities(&self, name: &str) -> Option<ChannelCapabilitiesFile> {
        let cap_path = self
            .wasm_channels_dir
            .join(format!("{}.capabilities.json", name));
        let bytes = tokio::fs::read(cap_path).await.ok()?;
        ChannelCapabilitiesFile::from_bytes(&bytes).ok()
    }

    fn setup_fields_setting_key(name: &str) -> String {
        format!("extensions.{name}.setup_fields")
    }

    async fn load_tool_setup_fields(
        &self,
        name: &str,
    ) -> Result<HashMap<String, String>, ExtensionError> {
        let Some(store) = &self.store else {
            return Ok(HashMap::new());
        };
        let key = Self::setup_fields_setting_key(name);
        match store.get_setting(&self.user_id, &key).await {
            Ok(Some(value)) => serde_json::from_value(value)
                .map_err(|e| ExtensionError::Other(format!("Invalid setup fields JSON: {}", e))),
            Ok(None) => Ok(HashMap::new()),
            Err(e) => Err(ExtensionError::Other(e.to_string())),
        }
    }

    async fn save_tool_setup_fields(
        &self,
        name: &str,
        fields: &HashMap<String, String>,
    ) -> Result<(), ExtensionError> {
        let store = self.store.as_ref().ok_or_else(|| {
            ExtensionError::Other(
                "Settings store unavailable for setup field persistence".to_string(),
            )
        })?;
        let key = Self::setup_fields_setting_key(name);
        let value = serde_json::to_value(fields)
            .map_err(|e| ExtensionError::Other(format!("Failed to encode setup fields: {}", e)))?;
        store
            .set_setting(&self.user_id, &key, &value)
            .await
            .map_err(|e| ExtensionError::Other(e.to_string()))
    }

    fn is_allowed_setup_setting_path(name: &str, setting_path: &str) -> bool {
        let namespaced_prefix = format!("extensions.{name}.");
        setting_path.starts_with(&namespaced_prefix)
            || ALLOWED_GLOBAL_SETUP_SETTING_PATHS.contains(&setting_path)
    }

    fn validate_setup_setting_path(name: &str, setting_path: &str) -> Result<(), ExtensionError> {
        if Self::is_allowed_setup_setting_path(name, setting_path) {
            Ok(())
        } else {
            Err(ExtensionError::Other(format!(
                "Invalid setting_path '{}' for extension '{}'",
                setting_path, name
            )))
        }
    }

    fn setting_value_is_present(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Null => false,
            serde_json::Value::String(s) => !s.trim().is_empty(),
            serde_json::Value::Array(a) => !a.is_empty(),
            serde_json::Value::Object(o) => !o.is_empty(),
            _ => true,
        }
    }

    async fn is_tool_setup_field_provided(
        &self,
        name: &str,
        field: &ToolFieldSetupSchema,
        saved_fields: &HashMap<String, String>,
    ) -> bool {
        if saved_fields
            .get(&field.name)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        {
            return true;
        }

        if let Some(setting_path) = &field.setting_path
            && let Some(store) = &self.store
            && let Ok(Some(value)) = store.get_setting(&self.user_id, setting_path).await
        {
            return Self::setting_value_is_present(&value);
        }

        let _ = name;
        false
    }

    async fn check_tool_auth_status(&self, name: &str, user_id: &str) -> ToolAuthState {
        let Some(cap) = self.load_tool_capabilities(name).await else {
            return ToolAuthState::NoAuth;
        };

        let saved_fields = self.load_tool_setup_fields(name).await.unwrap_or_default();

        if let Some(setup) = &cap.setup {
            let secrets_ready = futures::future::join_all(
                setup
                    .required_secrets
                    .iter()
                    .filter(|secret| !secret.optional)
                    .map(|secret| self.secrets.exists(user_id, &secret.name)),
            )
            .await
            .into_iter()
            .all(|result| result.unwrap_or(false));
            let fields_ready = futures::future::join_all(
                setup
                    .required_fields
                    .iter()
                    .filter(|field| !field.optional)
                    .map(|field| self.is_tool_setup_field_provided(name, field, &saved_fields)),
            )
            .await
            .into_iter()
            .all(|present| present);
            if !secrets_ready || !fields_ready {
                return ToolAuthState::NeedsSetup;
            }
        }

        let Some(auth) = &cap.auth else {
            return ToolAuthState::NoAuth;
        };

        if let Some(env_var) = &auth.env_var
            && std::env::var(env_var)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .is_some()
        {
            return ToolAuthState::Ready;
        }

        if self
            .secrets
            .exists(user_id, &auth.secret_name)
            .await
            .unwrap_or(false)
        {
            ToolAuthState::Ready
        } else {
            ToolAuthState::NeedsAuth
        }
    }

    async fn check_channel_auth_status(&self, name: &str, user_id: &str) -> ToolAuthState {
        let Some(cap) = self.load_channel_capabilities(name).await else {
            return ToolAuthState::NoAuth;
        };

        let required: Vec<_> = cap
            .setup
            .required_secrets
            .iter()
            .filter(|secret| !secret.optional)
            .collect();
        if required.is_empty() {
            return ToolAuthState::NoAuth;
        }

        let all_present = futures::future::join_all(
            required
                .iter()
                .map(|secret| self.secrets.exists(user_id, &secret.name)),
        )
        .await
        .into_iter()
        .all(|result| result.unwrap_or(false));

        if all_present {
            ToolAuthState::Ready
        } else {
            ToolAuthState::NeedsSetup
        }
    }

    async fn unregister_hook_prefix(&self, prefix: &str) -> usize {
        let Some(hooks) = &self.hooks else {
            return 0;
        };
        let names = hooks.list().await;
        let mut removed = 0;
        for name in names {
            if name.starts_with(prefix) && hooks.unregister(&name).await {
                removed += 1;
            }
        }
        removed
    }

    fn tool_secret_names(cap: &CapabilitiesFile) -> HashSet<String> {
        let mut names = HashSet::new();
        if let Some(auth) = &cap.auth {
            names.insert(auth.secret_name.to_lowercase());
        }
        if let Some(setup) = &cap.setup {
            names.extend(
                setup
                    .required_secrets
                    .iter()
                    .map(|secret| secret.name.to_lowercase()),
            );
        }
        if let Some(http) = &cap.http {
            names.extend(
                http.credentials
                    .values()
                    .map(|credential| credential.secret_name.to_lowercase()),
            );
        }
        names
    }

    async fn upgrade_one_tool(&self, name: &str) -> UpgradeOutcome {
        let cap_path = self
            .wasm_tools_dir
            .join(format!("{}.capabilities.json", name));
        let declared_wit = if cap_path.exists() {
            tokio::fs::read(&cap_path)
                .await
                .ok()
                .and_then(|bytes| CapabilitiesFile::from_bytes(&bytes).ok())
                .and_then(|cap| cap.wit_version)
        } else {
            None
        };

        if check_wit_version_compat(
            name,
            declared_wit.as_deref(),
            crate::tools::wasm::WIT_TOOL_VERSION,
        )
        .is_ok()
        {
            return UpgradeOutcome {
                name: name.to_string(),
                kind: ExtensionKind::WasmTool,
                status: "already_up_to_date".to_string(),
                detail: format!(
                    "WIT {} matches host WIT {}",
                    declared_wit.as_deref().unwrap_or("unknown"),
                    crate::tools::wasm::WIT_TOOL_VERSION
                ),
            };
        }

        let Some(entry) = self
            .registry
            .get_with_kind(name, Some(ExtensionKind::WasmTool))
            .await
        else {
            return UpgradeOutcome {
                name: name.to_string(),
                kind: ExtensionKind::WasmTool,
                status: "not_in_registry".to_string(),
                detail: "Extension is not in the registry.".to_string(),
            };
        };

        let wasm_path = self.wasm_tools_dir.join(format!("{}.wasm", name));
        if wasm_path.exists() {
            let _ = tokio::fs::remove_file(&wasm_path).await;
        }
        if cap_path.exists() {
            let _ = tokio::fs::remove_file(&cap_path).await;
        }

        match self.install_from_entry(&entry, &self.user_id).await {
            Ok(_) => UpgradeOutcome {
                name: name.to_string(),
                kind: ExtensionKind::WasmTool,
                status: "upgraded".to_string(),
                detail: format!(
                    "Upgraded from WIT {} to host WIT {}",
                    declared_wit.as_deref().unwrap_or("unknown"),
                    crate::tools::wasm::WIT_TOOL_VERSION
                ),
            },
            Err(error) => UpgradeOutcome {
                name: name.to_string(),
                kind: ExtensionKind::WasmTool,
                status: "failed".to_string(),
                detail: error.to_string(),
            },
        }
    }
}

fn infer_kind_from_url(url: &str) -> ExtensionKind {
    if url.ends_with(".wasm") || url.ends_with(".tar.gz") {
        ExtensionKind::WasmTool
    } else {
        ExtensionKind::McpServer
    }
}


#[allow(dead_code)]
fn normalize_hosted_callback_url(callback_url: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(callback_url) {
        let trimmed_path = parsed.path().trim_end_matches('/');
        let normalized = if trimmed_path.is_empty() {
            "/oauth/callback".to_string()
        } else if trimmed_path.ends_with("/oauth/callback") {
            trimmed_path.to_string()
        } else {
            format!("{trimmed_path}/oauth/callback")
        };
        parsed.set_path(&normalized);
        parsed.to_string()
    } else {
        callback_url.to_string()
    }
}

#[allow(dead_code)]
fn _ensure_path(_path: &Path) {}
