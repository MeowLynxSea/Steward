//! Desktop-first extension manager.
//!
//! Desktop/Tauri IPC remains the primary product surface, while optional
//! WASM channels can be activated as secondary ingress/egress adapters.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use chrono::Utc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::channels::ChannelManager;
use crate::channels::wasm::{ChannelCapabilitiesFile, WasmChannelLoader, WasmChannelRuntime};
use crate::extensions::discovery::OnlineDiscovery;
use crate::extensions::registry::ExtensionRegistry;
use crate::extensions::setup_schema::{SecretFieldInfo, SetupFieldInfo};
use crate::extensions::{
    ActivateResult, AuthResult, ConfigureResult, ExtensionError, ExtensionKind, ExtensionSource,
    InstallResult, InstalledExtension, RegistryEntry, ResultSource, SearchResult, ToolAuthState,
    UpgradeOutcome, UpgradeResult,
};
use crate::hooks::HookRegistry;
use crate::ipc::{McpActivityItemResponse, McpRootGrantResponse};
use crate::llm::{ChatMessage, CompletionRequest, ContentPart, ImageUrl, LlmProvider, Role};
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::task_runtime::{TaskMode, TaskOperation, TaskPendingApproval, TaskRuntime};
use crate::tools::ToolRegistry;
use crate::tools::mcp::auth::{authorize_mcp_server, is_authenticated};
use crate::tools::mcp::config::{McpServerConfig, McpServersFile};
use crate::tools::mcp::create_client_from_config;
use crate::tools::mcp::session::McpSessionManager;
use crate::tools::mcp::transport::McpInboundMessage;
use crate::tools::mcp::{
    CompleteResult, CompletionReference, GetPromptResult, McpClient, McpElicitationRequest,
    McpElicitationResult, McpPrimitiveSchemaDefinition, McpProcessManager, McpPrompt, McpResource,
    McpResourceTemplate, McpSamplingContentBlock, McpSamplingRequest, McpSamplingResult, McpTool,
    ReadResourceResult,
};
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
    "backends",
    "major_backend_id",
    "cheap_backend_id",
    "cheap_model_uses_primary",
];
const MCP_ROOTS_SETTINGS_PREFIX: &str = "mcp.roots.";
const MCP_SUBSCRIPTIONS_SETTINGS_PREFIX: &str = "mcp.subscriptions.";
const MCP_NEGOTIATED_SETTINGS_PREFIX: &str = "mcp.negotiated.";
const MCP_HEALTH_CHECK_SETTINGS_PREFIX: &str = "mcp.health_check.";
const MCP_ACTIVITY_SETTINGS_KEY: &str = "mcp.activity";
const MCP_ACTIVITY_LIMIT: usize = 100;

#[derive(Clone)]
struct PendingMcpSamplingRequest {
    server_name: String,
    request_id: serde_json::Value,
    request: McpSamplingRequest,
}

#[derive(Clone)]
struct PendingMcpElicitationRequest {
    server_name: String,
    request_id: serde_json::Value,
    request: McpElicitationRequest,
}

#[derive(Clone)]
struct McpRuntimeContext {
    mcp_session_manager: Arc<McpSessionManager>,
    mcp_process_manager: Arc<McpProcessManager>,
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    tool_registry: Arc<ToolRegistry>,
    store: Option<Arc<dyn crate::db::Database>>,
    owner_id: String,
    runtime_llm: Arc<RwLock<Option<Arc<dyn LlmProvider>>>>,
    task_runtime: Arc<RwLock<Option<Arc<TaskRuntime>>>>,
    pending_sampling_requests: Arc<RwLock<HashMap<Uuid, PendingMcpSamplingRequest>>>,
    pending_elicitation_requests: Arc<RwLock<HashMap<Uuid, PendingMcpElicitationRequest>>>,
    mcp_clients: Arc<RwLock<HashMap<String, Arc<McpClient>>>>,
    reconnecting_servers: Arc<RwLock<HashSet<String>>>,
}

pub struct ExtensionManager {
    registry: ExtensionRegistry,
    discovery: OnlineDiscovery,
    mcp_session_manager: Arc<McpSessionManager>,
    mcp_process_manager: Arc<McpProcessManager>,
    mcp_clients: Arc<RwLock<HashMap<String, Arc<McpClient>>>>,
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
    runtime_llm: Arc<RwLock<Option<Arc<dyn LlmProvider>>>>,
    task_runtime: Arc<RwLock<Option<Arc<TaskRuntime>>>>,
    pending_sampling_requests: Arc<RwLock<HashMap<Uuid, PendingMcpSamplingRequest>>>,
    pending_elicitation_requests: Arc<RwLock<HashMap<Uuid, PendingMcpElicitationRequest>>>,
    reconnecting_servers: Arc<RwLock<HashSet<String>>>,
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
            mcp_clients: Arc::new(RwLock::new(HashMap::new())),
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
            runtime_llm: Arc::new(RwLock::new(None)),
            task_runtime: Arc::new(RwLock::new(None)),
            pending_sampling_requests: Arc::new(RwLock::new(HashMap::new())),
            pending_elicitation_requests: Arc::new(RwLock::new(HashMap::new())),
            reconnecting_servers: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    pub async fn inject_registry_entry(&self, entry: RegistryEntry) {
        self.registry.cache_discovered(vec![entry]).await;
    }

    pub(crate) async fn inject_mcp_client(&self, name: String, client: Arc<McpClient>) {
        if let Err(error) =
            Self::sync_mcp_tools_with_registry(&self.tool_registry, &name, &client).await
        {
            tracing::warn!(server = %name, %error, "Failed to sync injected MCP client tools");
        }
        self.spawn_mcp_inbound_listener(name.clone(), Arc::clone(&client));
        self.mcp_clients.write().await.insert(name, client);
    }

    pub(crate) async fn notification_target_for_channel(&self, _name: &str) -> Option<String> {
        Some(self.user_id.clone())
    }

    pub fn secrets(&self) -> &Arc<dyn SecretsStore + Send + Sync> {
        &self.secrets
    }

    pub async fn bind_runtime_services(
        &self,
        llm: Arc<dyn LlmProvider>,
        task_runtime: Arc<TaskRuntime>,
    ) {
        *self.runtime_llm.write().await = Some(llm);
        *self.task_runtime.write().await = Some(task_runtime);
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
            return self
                .active_channel_names
                .read()
                .await
                .iter()
                .cloned()
                .collect();
        };

        match store
            .get_setting(user_id, "extensions.active_wasm_channels")
            .await
        {
            Ok(Some(value)) => serde_json::from_value::<Vec<String>>(value).unwrap_or_default(),
            Ok(None) | Err(_) => self
                .active_channel_names
                .read()
                .await
                .iter()
                .cloned()
                .collect(),
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
            if let Ok(channels) =
                crate::channels::wasm::discover_channels(&self.wasm_channels_dir).await
            {
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
                    .or_else(|| {
                        registry_entry
                            .as_ref()
                            .and_then(|entry| entry.version.clone())
                    });
                    let description = if let Some(cap_path) = &discovered.capabilities_path {
                        tokio::fs::read(cap_path)
                            .await
                            .ok()
                            .and_then(|bytes| ChannelCapabilitiesFile::from_bytes(&bytes).ok())
                            .and_then(|cap| cap.description)
                    } else {
                        registry_entry
                            .as_ref()
                            .map(|entry| entry.description.clone())
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

    pub async fn list_mcp_server_configs(
        &self,
        user_id: &str,
    ) -> Result<Vec<McpServerConfig>, ExtensionError> {
        Ok(self.load_mcp_servers(user_id).await?.servers)
    }

    pub async fn upsert_mcp_server(
        &self,
        user_id: &str,
        config: McpServerConfig,
    ) -> Result<McpServerConfig, ExtensionError> {
        config
            .validate()
            .map_err(|e| ExtensionError::Config(e.to_string()))?;
        if let Some(store) = &self.store {
            let mut servers = self.load_mcp_servers(user_id).await?;
            servers.upsert(config.clone());
            crate::tools::mcp::config::save_mcp_servers_to_db(store.as_ref(), user_id, &servers)
                .await
                .map_err(|e| ExtensionError::Config(e.to_string()))?;
        } else {
            crate::tools::mcp::config::add_mcp_server(config.clone())
                .await
                .map_err(|e| ExtensionError::Config(e.to_string()))?;
        }

        if !config.enabled {
            self.mcp_clients.write().await.remove(&config.name);
            self.reconnecting_servers.write().await.remove(&config.name);
            let prefix = format!("{}_", config.name);
            let stale_tools: Vec<String> = self
                .tool_registry
                .list()
                .await
                .into_iter()
                .filter(|tool| tool.starts_with(&prefix))
                .collect();
            for tool in stale_tools {
                self.tool_registry.unregister(&tool).await;
            }
            Self::fail_pending_requests_for_server(
                &self.task_runtime,
                &self.pending_sampling_requests,
                &self.pending_elicitation_requests,
                &config.name,
                "MCP server was disabled before the request could complete",
            )
            .await;
        }

        Ok(config)
    }

    pub async fn test_mcp_server(&self, name: &str, user_id: &str) -> Result<(), ExtensionError> {
        let client = self.ensure_mcp_client(name, user_id).await?;
        client
            .test_connection()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
        self.persist_mcp_health_check(name).await;
        Ok(())
    }

    pub async fn list_mcp_tools(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<Vec<McpTool>, ExtensionError> {
        let client = self.ensure_mcp_client(name, user_id).await?;
        client
            .list_tools()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))
    }

    pub async fn list_mcp_resources(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<Vec<McpResource>, ExtensionError> {
        let client = self.ensure_mcp_client(name, user_id).await?;
        client
            .list_resources()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))
    }

    pub async fn read_mcp_resource(
        &self,
        name: &str,
        user_id: &str,
        uri: &str,
    ) -> Result<ReadResourceResult, ExtensionError> {
        let client = self.ensure_mcp_client(name, user_id).await?;
        client
            .read_resource(uri)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))
    }

    pub async fn save_mcp_resource_snapshot(
        &self,
        name: &str,
        user_id: &str,
        uri: &str,
    ) -> Result<PathBuf, ExtensionError> {
        let resource = self.read_mcp_resource(name, user_id, uri).await?;
        let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
        let snapshot_dir = crate::bootstrap::steward_base_dir()
            .join("mcp-snapshots")
            .join(Self::sanitize_snapshot_segment(name))
            .join(format!(
                "{}-{}",
                timestamp,
                Self::sanitize_snapshot_segment(uri)
            ));
        tokio::fs::create_dir_all(&snapshot_dir)
            .await
            .map_err(|e| ExtensionError::Other(e.to_string()))?;

        let mut saved_files = Vec::new();
        for (index, content) in resource.contents.iter().enumerate() {
            match content {
                crate::tools::mcp::ResourceContents::Text(text) => {
                    let filename = format!(
                        "{:03}{}",
                        index + 1,
                        Self::extension_for_mime(text.mime_type.as_deref()).unwrap_or(".txt")
                    );
                    let path = snapshot_dir.join(&filename);
                    tokio::fs::write(&path, text.text.as_bytes())
                        .await
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;
                    saved_files.push(filename);
                }
                crate::tools::mcp::ResourceContents::Blob(blob) => {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(blob.blob.as_bytes())
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;
                    let filename = format!(
                        "{:03}{}",
                        index + 1,
                        Self::extension_for_mime(blob.mime_type.as_deref()).unwrap_or(".bin")
                    );
                    let path = snapshot_dir.join(&filename);
                    tokio::fs::write(&path, bytes)
                        .await
                        .map_err(|e| ExtensionError::Other(e.to_string()))?;
                    saved_files.push(filename);
                }
            }
        }

        let manifest = serde_json::json!({
            "server_name": name,
            "uri": uri,
            "saved_at": Utc::now(),
            "content_count": resource.contents.len(),
            "files": saved_files,
        });
        tokio::fs::write(
            snapshot_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)
                .map_err(|e| ExtensionError::Other(e.to_string()))?,
        )
        .await
        .map_err(|e| ExtensionError::Other(e.to_string()))?;

        Self::record_mcp_activity_with_store(
            self.store.as_ref(),
            &self.user_id,
            name,
            "snapshot",
            "Saved MCP resource snapshot",
            Some(snapshot_dir.display().to_string()),
        )
        .await;

        Ok(snapshot_dir)
    }

    pub async fn list_mcp_resource_templates(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<Vec<McpResourceTemplate>, ExtensionError> {
        let client = self.ensure_mcp_client(name, user_id).await?;
        client
            .list_resource_templates()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))
    }

    pub async fn list_mcp_prompts(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<Vec<McpPrompt>, ExtensionError> {
        let client = self.ensure_mcp_client(name, user_id).await?;
        client
            .list_prompts()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))
    }

    pub async fn get_mcp_prompt(
        &self,
        name: &str,
        user_id: &str,
        prompt_name: &str,
        arguments: Option<HashMap<String, String>>,
    ) -> Result<GetPromptResult, ExtensionError> {
        let client = self.ensure_mcp_client(name, user_id).await?;
        client
            .get_prompt(prompt_name, arguments)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))
    }

    pub async fn complete_mcp_argument(
        &self,
        name: &str,
        user_id: &str,
        reference: CompletionReference,
        argument_name: &str,
        value: &str,
        context_arguments: Option<HashMap<String, String>>,
    ) -> Result<CompleteResult, ExtensionError> {
        let client = self.ensure_mcp_client(name, user_id).await?;
        client
            .complete(reference, argument_name, value, context_arguments)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))
    }

    pub async fn subscribe_mcp_resource(
        &self,
        name: &str,
        user_id: &str,
        uri: &str,
    ) -> Result<(), ExtensionError> {
        let client = self.ensure_mcp_client(name, user_id).await?;
        client
            .subscribe_resource(uri)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
        let mut subscriptions = Self::load_mcp_resource_subscriptions_from_store(
            self.store.as_ref(),
            &self.user_id,
            name,
        )
        .await;
        if !subscriptions.iter().any(|existing| existing == uri) {
            subscriptions.push(uri.to_string());
            subscriptions.sort();
            self.save_mcp_resource_subscriptions_to_store(name, &subscriptions)
                .await?;
        }
        Ok(())
    }

    pub async fn unsubscribe_mcp_resource(
        &self,
        name: &str,
        user_id: &str,
        uri: &str,
    ) -> Result<(), ExtensionError> {
        let client = self.ensure_mcp_client(name, user_id).await?;
        client
            .unsubscribe_resource(uri)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
        let mut subscriptions = Self::load_mcp_resource_subscriptions_from_store(
            self.store.as_ref(),
            &self.user_id,
            name,
        )
        .await;
        subscriptions.retain(|existing| existing != uri);
        self.save_mcp_resource_subscriptions_to_store(name, &subscriptions)
            .await?;
        Ok(())
    }

    pub async fn notify_mcp_roots_changed(&self, name: &str) -> Result<(), ExtensionError> {
        let Some(client) = self.mcp_clients.read().await.get(name).cloned() else {
            return Ok(());
        };
        client
            .send_notification("notifications/roots/list_changed", None)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))
    }

    pub async fn respond_mcp_sampling(
        &self,
        task_id: Uuid,
        action: &str,
        request_override: Option<McpSamplingRequest>,
        generated_text: Option<String>,
    ) -> Result<crate::task_runtime::TaskRecord, ExtensionError> {
        let pending = self
            .pending_sampling_requests
            .read()
            .await
            .get(&task_id)
            .cloned()
            .ok_or_else(|| {
                ExtensionError::ActivationFailed(
                    "No pending MCP sampling request found".to_string(),
                )
            })?;
        let task_runtime = self
            .task_runtime
            .read()
            .await
            .clone()
            .ok_or_else(|| ExtensionError::Other("Task runtime not available".to_string()))?;

        match action {
            "generate" => {
                let effective_request = request_override.unwrap_or_else(|| pending.request.clone());
                let preview = self
                    .generate_mcp_sampling_preview(&effective_request)
                    .await?;
                let metadata = Self::sampling_task_metadata(
                    &pending.server_name,
                    &effective_request,
                    Some(&preview),
                );
                let task = task_runtime
                    .update_result_metadata(task_id, metadata)
                    .await
                    .ok_or_else(|| {
                        ExtensionError::ActivationFailed("MCP task not found".to_string())
                    })?;
                Self::record_mcp_activity_with_store(
                    self.store.as_ref(),
                    &self.user_id,
                    &pending.server_name,
                    "sampling",
                    "Generated MCP sampling preview",
                    None,
                )
                .await;
                Ok(task)
            }
            "approve" => {
                let effective_request = request_override.unwrap_or_else(|| pending.request.clone());
                let result = if let Some(text) = generated_text {
                    McpSamplingResult {
                        role: "assistant".to_string(),
                        content: McpSamplingContentBlock::Text {
                            text,
                            annotations: None,
                        },
                        model: Some(
                            self.runtime_llm
                                .read()
                                .await
                                .as_ref()
                                .map(|llm| llm.active_model_name()),
                        )
                        .flatten(),
                        stop_reason: Some("endTurn".to_string()),
                    }
                } else if let Some(existing) =
                    task_runtime.get_task(task_id).await.and_then(|task| {
                        task.result_metadata
                            .and_then(|metadata| metadata.get("preview").cloned())
                            .and_then(|value| {
                                serde_json::from_value::<McpSamplingResult>(value).ok()
                            })
                    })
                {
                    existing
                } else {
                    self.generate_mcp_sampling_preview(&effective_request)
                        .await?
                };

                let payload = serde_json::to_value(&result)
                    .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
                let task_metadata = Self::sampling_task_metadata(
                    &pending.server_name,
                    &effective_request,
                    Some(&result),
                );
                let task = self
                    .finish_pending_sampling(task_id, &pending, payload, task_metadata)
                    .await?;
                Self::record_mcp_activity_with_store(
                    self.store.as_ref(),
                    &self.user_id,
                    &pending.server_name,
                    "sampling",
                    "Approved MCP sampling response",
                    None,
                )
                .await;
                Ok(task)
            }
            "decline" => {
                let task = self
                    .reject_pending_sampling(task_id, &pending, false)
                    .await?;
                Self::record_mcp_activity_with_store(
                    self.store.as_ref(),
                    &self.user_id,
                    &pending.server_name,
                    "sampling",
                    "Declined MCP sampling response",
                    None,
                )
                .await;
                Ok(task)
            }
            "cancel" => {
                self.notify_mcp_request_cancelled(
                    &pending.server_name,
                    &pending.request_id,
                    "Cancelled from MCP panel",
                )
                .await;
                let task = self
                    .reject_pending_sampling(task_id, &pending, true)
                    .await?;
                Self::record_mcp_activity_with_store(
                    self.store.as_ref(),
                    &self.user_id,
                    &pending.server_name,
                    "sampling",
                    "Cancelled MCP sampling response",
                    None,
                )
                .await;
                Ok(task)
            }
            other => Err(ExtensionError::ActivationFailed(format!(
                "Unsupported MCP sampling action '{other}'"
            ))),
        }
    }

    pub async fn respond_mcp_elicitation(
        &self,
        task_id: Uuid,
        action: &str,
        content: Option<HashMap<String, serde_json::Value>>,
    ) -> Result<crate::task_runtime::TaskRecord, ExtensionError> {
        let pending = self
            .pending_elicitation_requests
            .read()
            .await
            .get(&task_id)
            .cloned()
            .ok_or_else(|| {
                ExtensionError::ActivationFailed(
                    "No pending MCP elicitation request found".to_string(),
                )
            })?;
        let task_runtime = self
            .task_runtime
            .read()
            .await
            .clone()
            .ok_or_else(|| ExtensionError::Other("Task runtime not available".to_string()))?;

        match action {
            "accept" => {
                let content = content.unwrap_or_default();
                Self::validate_mcp_elicitation_content(&pending.request, &content)?;
                let result = McpElicitationResult {
                    action: "accept".to_string(),
                    content: Some(content.clone()),
                };
                let payload = serde_json::to_value(&result)
                    .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
                let task_metadata = Self::elicitation_task_metadata(
                    &pending.server_name,
                    &pending.request,
                    Some(&content),
                    Some("accept"),
                );
                self.pending_elicitation_requests
                    .write()
                    .await
                    .remove(&task_id);
                let client = self
                    .mcp_clients
                    .read()
                    .await
                    .get(&pending.server_name)
                    .cloned()
                    .ok_or_else(|| {
                        ExtensionError::ActivationFailed("MCP client not found".to_string())
                    })?;
                client
                    .respond_success(pending.request_id.clone(), payload)
                    .await
                    .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
                task_runtime
                    .mark_completed_with_result(task_id, Some(task_metadata))
                    .await;
                let task = task_runtime.get_task(task_id).await.ok_or_else(|| {
                    ExtensionError::ActivationFailed("MCP task not found".to_string())
                })?;
                Self::record_mcp_activity_with_store(
                    self.store.as_ref(),
                    &self.user_id,
                    &pending.server_name,
                    "elicitation",
                    "Accepted MCP elicitation response",
                    None,
                )
                .await;
                Ok(task)
            }
            "decline" | "cancel" => {
                if action == "cancel" {
                    self.notify_mcp_request_cancelled(
                        &pending.server_name,
                        &pending.request_id,
                        "Cancelled from MCP panel",
                    )
                    .await;
                }
                let result = McpElicitationResult {
                    action: action.to_string(),
                    content: None,
                };
                let payload = serde_json::to_value(&result)
                    .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
                let task_metadata = Self::elicitation_task_metadata(
                    &pending.server_name,
                    &pending.request,
                    None,
                    Some(action),
                );
                self.pending_elicitation_requests
                    .write()
                    .await
                    .remove(&task_id);
                let client = self
                    .mcp_clients
                    .read()
                    .await
                    .get(&pending.server_name)
                    .cloned()
                    .ok_or_else(|| {
                        ExtensionError::ActivationFailed("MCP client not found".to_string())
                    })?;
                client
                    .respond_success(pending.request_id.clone(), payload)
                    .await
                    .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
                if action == "cancel" {
                    task_runtime
                        .mark_cancelled(task_id, "Cancelled from MCP panel")
                        .await;
                } else {
                    task_runtime
                        .mark_rejected(task_id, "Declined from MCP panel")
                        .await;
                }
                let task = task_runtime
                    .update_result_metadata(task_id, task_metadata)
                    .await
                    .ok_or_else(|| {
                        ExtensionError::ActivationFailed("MCP task not found".to_string())
                    })?;
                Self::record_mcp_activity_with_store(
                    self.store.as_ref(),
                    &self.user_id,
                    &pending.server_name,
                    "elicitation",
                    if action == "cancel" {
                        "Cancelled MCP elicitation response"
                    } else {
                        "Declined MCP elicitation response"
                    },
                    None,
                )
                .await;
                Ok(task)
            }
            other => Err(ExtensionError::ActivationFailed(format!(
                "Unsupported MCP elicitation action '{other}'"
            ))),
        }
    }

    pub async fn cancel_pending_mcp_task(
        &self,
        task_id: Uuid,
    ) -> Result<Option<crate::task_runtime::TaskRecord>, ExtensionError> {
        if self
            .pending_sampling_requests
            .read()
            .await
            .contains_key(&task_id)
        {
            return self
                .respond_mcp_sampling(task_id, "cancel", None, None)
                .await
                .map(Some);
        }

        if self
            .pending_elicitation_requests
            .read()
            .await
            .contains_key(&task_id)
        {
            return self
                .respond_mcp_elicitation(task_id, "cancel", None)
                .await
                .map(Some);
        }

        Ok(None)
    }

    async fn notify_mcp_request_cancelled(
        &self,
        server_name: &str,
        request_id: &serde_json::Value,
        reason: &str,
    ) {
        let Some(client) = self.mcp_clients.read().await.get(server_name).cloned() else {
            return;
        };
        let params = serde_json::json!({
            "requestId": request_id,
            "reason": reason,
        });
        if let Err(error) = client
            .send_notification("notifications/cancelled", Some(params))
            .await
        {
            tracing::debug!(
                server = %server_name,
                %error,
                "Failed to send MCP cancelled notification"
            );
        }
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
                self.reconnecting_servers.write().await.remove(name);
                let failed_pending_count = Self::fail_pending_requests_for_server(
                    &self.task_runtime,
                    &self.pending_sampling_requests,
                    &self.pending_elicitation_requests,
                    name,
                    "MCP server was removed before the request could complete",
                )
                .await;
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
                    "Removed MCP server '{}' and {} tool(s){}",
                    name,
                    tool_names.len(),
                    if failed_pending_count > 0 {
                        format!(", failed {failed_pending_count} pending MCP task(s)")
                    } else {
                        String::new()
                    }
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
            if let Ok(channels) =
                crate::channels::wasm::discover_channels(&self.wasm_channels_dir).await
            {
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
                    value["allowed_paths"] = serde_json::json!(
                        cap.capabilities
                            .channel
                            .as_ref()
                            .map(|c| c.allowed_paths.clone())
                            .unwrap_or_default()
                    );
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
                let secrets =
                    futures::future::join_all(cap_file.setup.required_secrets.iter().map(
                        |secret| async move {
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
                        },
                    ))
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
                        ExtensionError::Other(format!("Channel '{}' has no setup secrets", name))
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
            return Ok(AuthResult::no_auth_required(
                name,
                ExtensionKind::WasmChannel,
            ));
        };

        let required: Vec<_> = cap
            .setup
            .required_secrets
            .iter()
            .filter(|secret| !secret.optional)
            .collect();
        if required.is_empty() {
            return Ok(AuthResult::no_auth_required(
                name,
                ExtensionKind::WasmChannel,
            ));
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
        let _ = self.cache_and_attach_mcp_client(name, client).await?;

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
                ExtensionError::ActivationFailed("WASM channel runtime not available".to_string())
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

        let loader = WasmChannelLoader::new(
            runtime,
            Arc::new(crate::pairing::PairingStore::new()),
            None,
            self.user_id.clone(),
        )
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
        let mut active = self
            .active_channel_names
            .read()
            .await
            .iter()
            .cloned()
            .collect::<Vec<_>>();
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
        if self
            .wasm_channels_dir
            .join(format!("{}.wasm", name))
            .exists()
        {
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

    fn mcp_roots_setting_key(server_name: &str) -> String {
        format!("{MCP_ROOTS_SETTINGS_PREFIX}{server_name}")
    }

    fn mcp_subscriptions_setting_key(server_name: &str) -> String {
        format!("{MCP_SUBSCRIPTIONS_SETTINGS_PREFIX}{server_name}")
    }

    fn mcp_negotiated_setting_key(server_name: &str) -> String {
        format!("{MCP_NEGOTIATED_SETTINGS_PREFIX}{server_name}")
    }

    fn mcp_health_check_setting_key(server_name: &str) -> String {
        format!("{MCP_HEALTH_CHECK_SETTINGS_PREFIX}{server_name}")
    }

    fn summarize_mcp_detail(params: &Option<serde_json::Value>) -> Option<String> {
        let Some(params) = params else {
            return None;
        };
        let detail = serde_json::to_string(params).ok()?;
        let max_chars = 240;
        if detail.chars().count() <= max_chars {
            return Some(detail);
        }
        let byte_offset = detail
            .char_indices()
            .nth(max_chars)
            .map(|(idx, _)| idx)
            .unwrap_or(detail.len());
        Some(format!("{}…", &detail[..byte_offset]))
    }

    async fn load_mcp_root_grants_from_store(
        store: Option<&Arc<dyn crate::db::Database>>,
        owner_id: &str,
        server_name: &str,
    ) -> Vec<McpRootGrantResponse> {
        let Some(store) = store else {
            return Vec::new();
        };
        match store
            .get_setting(owner_id, &Self::mcp_roots_setting_key(server_name))
            .await
        {
            Ok(Some(value)) => serde_json::from_value(value).unwrap_or_default(),
            Ok(None) | Err(_) => Vec::new(),
        }
    }

    async fn load_mcp_resource_subscriptions_from_store(
        store: Option<&Arc<dyn crate::db::Database>>,
        owner_id: &str,
        server_name: &str,
    ) -> Vec<String> {
        let Some(store) = store else {
            return Vec::new();
        };
        match store
            .get_setting(owner_id, &Self::mcp_subscriptions_setting_key(server_name))
            .await
        {
            Ok(Some(value)) => serde_json::from_value(value).unwrap_or_default(),
            Ok(None) | Err(_) => Vec::new(),
        }
    }

    async fn save_mcp_resource_subscriptions_to_store(
        &self,
        server_name: &str,
        subscriptions: &[String],
    ) -> Result<(), ExtensionError> {
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };
        let value = serde_json::to_value(subscriptions)
            .map_err(|e| ExtensionError::Other(e.to_string()))?;
        store
            .set_setting(
                &self.user_id,
                &Self::mcp_subscriptions_setting_key(server_name),
                &value,
            )
            .await
            .map_err(|e| ExtensionError::Other(e.to_string()))
    }

    async fn persist_mcp_health_check(&self, server_name: &str) {
        Self::persist_mcp_health_check_with_store(self.store.as_ref(), &self.user_id, server_name)
            .await;
    }

    async fn record_mcp_activity_with_store(
        store: Option<&Arc<dyn crate::db::Database>>,
        owner_id: &str,
        server_name: &str,
        kind: &str,
        title: impl Into<String>,
        detail: Option<String>,
    ) {
        let Some(store) = store else {
            return;
        };

        let mut items: Vec<McpActivityItemResponse> =
            match store.get_setting(owner_id, MCP_ACTIVITY_SETTINGS_KEY).await {
                Ok(Some(value)) => serde_json::from_value(value).unwrap_or_default(),
                Ok(None) | Err(_) => Vec::new(),
            };

        items.insert(
            0,
            McpActivityItemResponse {
                id: Uuid::new_v4().to_string(),
                server_name: server_name.to_string(),
                kind: kind.to_string(),
                title: title.into(),
                detail,
                created_at: Utc::now(),
            },
        );
        if items.len() > MCP_ACTIVITY_LIMIT {
            items.truncate(MCP_ACTIVITY_LIMIT);
        }

        let Ok(value) = serde_json::to_value(&items) else {
            return;
        };
        if let Err(error) = store
            .set_setting(owner_id, MCP_ACTIVITY_SETTINGS_KEY, &value)
            .await
        {
            tracing::warn!(%error, server = %server_name, "Failed to persist MCP activity");
        }
    }

    fn mcp_runtime_context(&self) -> McpRuntimeContext {
        McpRuntimeContext {
            mcp_session_manager: Arc::clone(&self.mcp_session_manager),
            mcp_process_manager: Arc::clone(&self.mcp_process_manager),
            secrets: Arc::clone(&self.secrets),
            tool_registry: Arc::clone(&self.tool_registry),
            store: self.store.clone(),
            owner_id: self.user_id.clone(),
            runtime_llm: Arc::clone(&self.runtime_llm),
            task_runtime: Arc::clone(&self.task_runtime),
            pending_sampling_requests: Arc::clone(&self.pending_sampling_requests),
            pending_elicitation_requests: Arc::clone(&self.pending_elicitation_requests),
            mcp_clients: Arc::clone(&self.mcp_clients),
            reconnecting_servers: Arc::clone(&self.reconnecting_servers),
        }
    }

    async fn restore_mcp_resource_subscriptions_with_store(
        store: Option<&Arc<dyn crate::db::Database>>,
        owner_id: &str,
        server_name: &str,
        client: &Arc<McpClient>,
    ) -> Result<(), ExtensionError> {
        let subscriptions =
            Self::load_mcp_resource_subscriptions_from_store(store, owner_id, server_name).await;
        for uri in subscriptions {
            client
                .subscribe_resource(&uri)
                .await
                .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
        }
        Ok(())
    }

    async fn sync_mcp_tools_with_registry(
        tool_registry: &Arc<ToolRegistry>,
        server_name: &str,
        client: &Arc<McpClient>,
    ) -> Result<(), ExtensionError> {
        let prefix = format!("{server_name}_");
        let existing: Vec<String> = tool_registry
            .list()
            .await
            .into_iter()
            .filter(|tool| tool.starts_with(&prefix))
            .collect();

        let tools = client
            .create_tools()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
        let new_names: HashSet<String> = tools.iter().map(|tool| tool.name().to_string()).collect();

        for stale in existing {
            if !new_names.contains(&stale) {
                tool_registry.unregister(&stale).await;
            }
        }

        for tool in tools {
            tool_registry.register(tool).await;
        }

        Ok(())
    }

    fn spawn_mcp_inbound_listener(&self, server_name: String, client: Arc<McpClient>) {
        Self::spawn_mcp_inbound_listener_with_context(
            server_name,
            client,
            self.mcp_runtime_context(),
        );
    }

    fn spawn_mcp_inbound_listener_with_context(
        server_name: String,
        client: Arc<McpClient>,
        ctx: McpRuntimeContext,
    ) {
        let Some(mut inbound) = client.subscribe_inbound() else {
            return;
        };

        tokio::spawn(async move {
            loop {
                match inbound.recv().await {
                    Ok(McpInboundMessage::Notification(notification)) => {
                        let detail = Self::summarize_mcp_detail(&notification.params);
                        match notification.method.as_str() {
                            "notifications/tools/list_changed" => {
                                client.invalidate_tools_cache().await;
                                match Self::sync_mcp_tools_with_registry(
                                    &ctx.tool_registry,
                                    &server_name,
                                    &client,
                                )
                                .await
                                {
                                    Ok(()) => {
                                        Self::record_mcp_activity_with_store(
                                            ctx.store.as_ref(),
                                            &ctx.owner_id,
                                            &server_name,
                                            "tools",
                                            "Refreshed MCP tool list",
                                            detail,
                                        )
                                        .await;
                                    }
                                    Err(error) => {
                                        Self::record_mcp_activity_with_store(
                                            ctx.store.as_ref(),
                                            &ctx.owner_id,
                                            &server_name,
                                            "error",
                                            "Failed to refresh MCP tool list",
                                            Some(error.to_string()),
                                        )
                                        .await;
                                    }
                                }
                            }
                            "notifications/resources/list_changed" => {
                                client.invalidate_resources_cache().await;
                                Self::record_mcp_activity_with_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                    "resource",
                                    "MCP resources changed",
                                    detail,
                                )
                                .await;
                            }
                            "notifications/prompts/list_changed" => {
                                client.invalidate_prompts_cache().await;
                                Self::record_mcp_activity_with_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                    "prompt",
                                    "MCP prompts changed",
                                    detail,
                                )
                                .await;
                            }
                            "notifications/resources/updated" => {
                                client.invalidate_resources_cache().await;
                                Self::record_mcp_activity_with_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                    "resource",
                                    "MCP resource updated",
                                    detail,
                                )
                                .await;
                            }
                            "notifications/progress" => {
                                Self::record_mcp_activity_with_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                    "progress",
                                    "MCP progress update",
                                    detail,
                                )
                                .await;
                            }
                            "notifications/message" => {
                                Self::record_mcp_activity_with_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                    "message",
                                    "MCP server message",
                                    detail,
                                )
                                .await;
                            }
                            "notifications/cancelled" => {
                                Self::record_mcp_activity_with_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                    "cancelled",
                                    "MCP request cancelled",
                                    detail,
                                )
                                .await;
                            }
                            "notifications/roots/list_changed" => {
                                Self::record_mcp_activity_with_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                    "roots",
                                    "MCP server requested roots refresh",
                                    detail,
                                )
                                .await;
                            }
                            other => {
                                Self::record_mcp_activity_with_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                    "notification",
                                    format!("Received MCP notification '{other}'"),
                                    detail,
                                )
                                .await;
                            }
                        }
                    }
                    Ok(McpInboundMessage::Request(request)) => {
                        match request.method.as_str() {
                            "roots/list" => {
                                let roots = Self::load_mcp_root_grants_from_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                )
                                .await
                                .into_iter()
                                .map(|root| {
                                    serde_json::json!({
                                        "uri": root.uri,
                                        "name": root.name,
                                    })
                                })
                                .collect::<Vec<_>>();

                                if let Err(error) = client
                                    .respond_success(
                                        request.id.clone(),
                                        serde_json::json!({ "roots": roots }),
                                    )
                                    .await
                                {
                                    Self::record_mcp_activity_with_store(
                                        ctx.store.as_ref(),
                                        &ctx.owner_id,
                                        &server_name,
                                        "error",
                                        "Failed to answer MCP roots/list",
                                        Some(error.to_string()),
                                    )
                                    .await;
                                } else {
                                    Self::record_mcp_activity_with_store(
                                        ctx.store.as_ref(),
                                        &ctx.owner_id,
                                        &server_name,
                                        "roots",
                                        "Answered MCP roots/list",
                                        None,
                                    )
                                    .await;
                                }
                            }
                            "sampling/createMessage" => {
                                let params = request
                                    .params
                                    .clone()
                                    .ok_or_else(|| {
                                        ExtensionError::ActivationFailed(
                                            "Missing sampling request params".to_string(),
                                        )
                                    })
                                    .and_then(|params| {
                                        serde_json::from_value::<McpSamplingRequest>(params)
                                            .map_err(|e| {
                                                ExtensionError::ActivationFailed(format!(
                                                    "Invalid sampling request: {e}"
                                                ))
                                            })
                                    });

                                match params {
                                    Ok(params) => {
                                        let preferred_mode = Self::mcp_sampling_mode_for_request(
                                            &ctx.task_runtime,
                                            &client,
                                        )
                                        .await;

                                        if preferred_mode == TaskMode::Yolo {
                                            if let Some(task_id) = Self::create_mcp_sampling_task(
                                                &ctx.task_runtime,
                                                &server_name,
                                                &params,
                                                TaskMode::Yolo,
                                            )
                                            .await
                                            {
                                                Self::record_mcp_activity_with_store(
                                                    ctx.store.as_ref(),
                                                    &ctx.owner_id,
                                                    &server_name,
                                                    "sampling",
                                                    "Auto-running MCP sampling in Yolo mode",
                                                    Some(format!("Task {}", task_id)),
                                                )
                                                .await;

                                                match Self::generate_mcp_sampling_preview_with_runtime(
                                                    &ctx.runtime_llm,
                                                    &params,
                                                )
                                                .await
                                                {
                                                    Ok(result) => {
                                                        let payload = match serde_json::to_value(&result) {
                                                            Ok(payload) => payload,
                                                            Err(error) => {
                                                                let _ = client
                                                                    .respond_error(
                                                                        request.id.clone(),
                                                                        -32603,
                                                                        &error.to_string(),
                                                                        None,
                                                                    )
                                                                    .await;
                                                                if let Some(runtime) =
                                                                    ctx.task_runtime.read().await.clone()
                                                                {
                                                                    runtime
                                                                        .mark_failed(task_id, error.to_string())
                                                                        .await;
                                                                }
                                                                continue;
                                                            }
                                                        };
                                                        let metadata = Self::sampling_task_metadata(
                                                            &server_name,
                                                            &params,
                                                            Some(&result),
                                                        );
                                                        match client
                                                            .respond_success(
                                                                request.id.clone(),
                                                                payload,
                                                            )
                                                            .await
                                                        {
                                                            Ok(()) => {
                                                                if let Some(runtime) =
                                                                    ctx.task_runtime.read().await.clone()
                                                                {
                                                                    runtime
                                                                        .mark_completed_with_result(
                                                                            task_id,
                                                                            Some(metadata),
                                                                        )
                                                                        .await;
                                                                }
                                                                Self::record_mcp_activity_with_store(
                                                                    ctx.store.as_ref(),
                                                                    &ctx.owner_id,
                                                                    &server_name,
                                                                    "sampling",
                                                                    "Returned MCP sampling response automatically",
                                                                    Some(format!("Task {}", task_id)),
                                                                )
                                                                .await;
                                                            }
                                                            Err(error) => {
                                                                if let Some(runtime) =
                                                                    ctx.task_runtime.read().await.clone()
                                                                {
                                                                    runtime
                                                                        .mark_failed(
                                                                            task_id,
                                                                            error.to_string(),
                                                                        )
                                                                        .await;
                                                                }
                                                                Self::record_mcp_activity_with_store(
                                                                    ctx.store.as_ref(),
                                                                    &ctx.owner_id,
                                                                    &server_name,
                                                                    "error",
                                                                    "Failed to return MCP sampling response",
                                                                    Some(error.to_string()),
                                                                )
                                                                .await;
                                                            }
                                                        }
                                                    }
                                                    Err(error) => {
                                                        let _ = client
                                                            .respond_error(
                                                                request.id.clone(),
                                                                -32603,
                                                                &error.to_string(),
                                                                None,
                                                            )
                                                            .await;
                                                        if let Some(runtime) =
                                                            ctx.task_runtime.read().await.clone()
                                                        {
                                                            runtime
                                                                .mark_failed(task_id, error.to_string())
                                                                .await;
                                                        }
                                                        Self::record_mcp_activity_with_store(
                                                            ctx.store.as_ref(),
                                                            &ctx.owner_id,
                                                            &server_name,
                                                            "error",
                                                            "Failed to auto-run MCP sampling request",
                                                            Some(error.to_string()),
                                                        )
                                                        .await;
                                                    }
                                                }
                                            } else {
                                                let _ = client
                                                    .respond_error(
                                                        request.id.clone(),
                                                        -32001,
                                                        "MCP sampling requires desktop task runtime support",
                                                        None,
                                                    )
                                                    .await;
                                            }
                                        } else {
                                            if let Some(task_id) = Self::create_mcp_sampling_task(
                                                &ctx.task_runtime,
                                                &server_name,
                                                &params,
                                                TaskMode::Ask,
                                            )
                                            .await
                                            {
                                                ctx.pending_sampling_requests.write().await.insert(
                                                    task_id,
                                                    PendingMcpSamplingRequest {
                                                        server_name: server_name.clone(),
                                                        request_id: request.id.clone(),
                                                        request: params.clone(),
                                                    },
                                                );
                                                Self::record_mcp_activity_with_store(
                                                    ctx.store.as_ref(),
                                                    &ctx.owner_id,
                                                    &server_name,
                                                    "sampling",
                                                    "Queued MCP sampling approval",
                                                    Some(format!("Task {}", task_id)),
                                                )
                                                .await;
                                            } else {
                                                let _ = client
                                                    .respond_error(
                                                        request.id.clone(),
                                                        -32001,
                                                        "MCP sampling requires desktop task runtime support",
                                                        None,
                                                    )
                                                    .await;
                                            }
                                        }
                                    }
                                    Err(error) => {
                                        let _ = client
                                            .respond_error(
                                                request.id.clone(),
                                                -32602,
                                                error.to_string(),
                                                None,
                                            )
                                            .await;
                                    }
                                }
                            }
                            "elicitation/create" => {
                                let params = request
                                    .params
                                    .clone()
                                    .ok_or_else(|| {
                                        ExtensionError::ActivationFailed(
                                            "Missing elicitation request params".to_string(),
                                        )
                                    })
                                    .and_then(|params| {
                                        serde_json::from_value::<McpElicitationRequest>(params)
                                            .map_err(|e| {
                                                ExtensionError::ActivationFailed(format!(
                                                    "Invalid elicitation request: {e}"
                                                ))
                                            })
                                    });

                                match params {
                                    Ok(params) => {
                                        if let Some(task_id) = Self::create_mcp_elicitation_task(
                                            &ctx.task_runtime,
                                            &server_name,
                                            &params,
                                        )
                                        .await
                                        {
                                            ctx.pending_elicitation_requests.write().await.insert(
                                                task_id,
                                                PendingMcpElicitationRequest {
                                                    server_name: server_name.clone(),
                                                    request_id: request.id.clone(),
                                                    request: params.clone(),
                                                },
                                            );
                                            Self::record_mcp_activity_with_store(
                                                ctx.store.as_ref(),
                                                &ctx.owner_id,
                                                &server_name,
                                                "elicitation",
                                                "Queued MCP elicitation approval",
                                                Some(format!("Task {}", task_id)),
                                            )
                                            .await;
                                        } else {
                                            let _ = client
                                            .respond_error(
                                                request.id.clone(),
                                                -32001,
                                                "MCP elicitation requires desktop task runtime support",
                                                None,
                                            )
                                            .await;
                                        }
                                    }
                                    Err(error) => {
                                        let _ = client
                                            .respond_error(
                                                request.id.clone(),
                                                -32602,
                                                error.to_string(),
                                                None,
                                            )
                                            .await;
                                    }
                                }
                            }
                            other => {
                                let _ = client
                                    .respond_error(
                                        request.id.clone(),
                                        -32601,
                                        format!("Unsupported MCP method '{other}'"),
                                        None,
                                    )
                                    .await;
                                Self::record_mcp_activity_with_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                    "request",
                                    format!("Rejected unsupported MCP request '{other}'"),
                                    Self::summarize_mcp_detail(&request.params),
                                )
                                .await;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        let removed_current = {
                            let mut clients = ctx.mcp_clients.write().await;
                            match clients.get(&server_name) {
                                Some(active_client) if Arc::ptr_eq(active_client, &client) => {
                                    clients.remove(&server_name);
                                    true
                                }
                                _ => false,
                            }
                        };

                        if removed_current {
                            Self::unregister_mcp_tools_for_server(&ctx.tool_registry, &server_name)
                                .await;
                        }
                        let failed_pending_count = Self::fail_pending_requests_for_server(
                            &ctx.task_runtime,
                            &ctx.pending_sampling_requests,
                            &ctx.pending_elicitation_requests,
                            &server_name,
                            "MCP connection closed before the request could complete",
                        )
                        .await;

                        Self::record_mcp_activity_with_store(
                            ctx.store.as_ref(),
                            &ctx.owner_id,
                            &server_name,
                            "connection",
                            if removed_current {
                                "MCP connection closed; client evicted until reconnect"
                            } else {
                                "MCP inbound stream closed"
                            },
                            (failed_pending_count > 0).then(|| {
                                format!(
                                    "Failed {failed_pending_count} pending MCP approval task(s)"
                                )
                            }),
                        )
                        .await;
                        if removed_current {
                            Self::spawn_background_mcp_reconnect(server_name.clone(), ctx.clone());
                        }
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(
                            server = %server_name,
                            skipped,
                            "Lagged while consuming inbound MCP messages"
                        );
                        Self::record_mcp_activity_with_store(
                            ctx.store.as_ref(),
                            &ctx.owner_id,
                            &server_name,
                            "warning",
                            "Lagged while processing MCP activity",
                            Some(format!("Skipped {skipped} inbound messages")),
                        )
                        .await;
                    }
                }
            }
        });
    }

    async fn attach_mcp_client_with_context(
        name: &str,
        client: McpClient,
        ctx: &McpRuntimeContext,
    ) -> Result<Arc<McpClient>, ExtensionError> {
        let client = Arc::new(client);
        let init_result = client
            .initialize()
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;
        Self::persist_mcp_negotiated_state_with_store(
            ctx.store.as_ref(),
            &ctx.owner_id,
            name,
            &init_result,
        )
        .await;
        Self::restore_mcp_resource_subscriptions_with_store(
            ctx.store.as_ref(),
            &ctx.owner_id,
            name,
            &client,
        )
        .await?;
        Self::sync_mcp_tools_with_registry(&ctx.tool_registry, name, &client).await?;
        ctx.mcp_clients
            .write()
            .await
            .insert(name.to_string(), Arc::clone(&client));
        Self::spawn_mcp_inbound_listener_with_context(
            name.to_string(),
            Arc::clone(&client),
            ctx.clone(),
        );
        Self::persist_mcp_health_check_with_store(ctx.store.as_ref(), &ctx.owner_id, name).await;
        Ok(client)
    }

    async fn cache_and_attach_mcp_client(
        &self,
        name: &str,
        client: McpClient,
    ) -> Result<Arc<McpClient>, ExtensionError> {
        Self::attach_mcp_client_with_context(name, client, &self.mcp_runtime_context()).await
    }

    async fn load_mcp_server_for_reconnect(
        store: Option<&Arc<dyn crate::db::Database>>,
        owner_id: &str,
        name: &str,
    ) -> Result<Option<McpServerConfig>, ExtensionError> {
        let servers = if let Some(store) = store {
            crate::tools::mcp::config::load_mcp_servers_from_db(store.as_ref(), owner_id)
                .await
                .map_err(|e| ExtensionError::Config(e.to_string()))?
        } else {
            crate::tools::mcp::config::load_mcp_servers()
                .await
                .map_err(|e| ExtensionError::Config(e.to_string()))?
        };
        Ok(servers.get(name).cloned())
    }

    async fn persist_mcp_negotiated_state_with_store(
        store: Option<&Arc<dyn crate::db::Database>>,
        owner_id: &str,
        server_name: &str,
        result: &crate::tools::mcp::InitializeResult,
    ) {
        let Some(store) = store else {
            return;
        };
        let value = serde_json::json!({
            "protocol_version": result.protocol_version,
            "capabilities": result.capabilities,
            "server_info": result.server_info,
            "instructions": result.instructions,
        });
        if let Err(error) = store
            .set_setting(
                owner_id,
                &Self::mcp_negotiated_setting_key(server_name),
                &value,
            )
            .await
        {
            tracing::warn!(%error, server = %server_name, "Failed to persist MCP negotiated state");
        }
    }

    async fn persist_mcp_health_check_with_store(
        store: Option<&Arc<dyn crate::db::Database>>,
        owner_id: &str,
        server_name: &str,
    ) {
        let Some(store) = store else {
            return;
        };
        if let Err(error) = store
            .set_setting(
                owner_id,
                &Self::mcp_health_check_setting_key(server_name),
                &serde_json::json!(Utc::now()),
            )
            .await
        {
            tracing::warn!(%error, server = %server_name, "Failed to persist MCP health check");
        }
    }

    fn spawn_background_mcp_reconnect(server_name: String, ctx: McpRuntimeContext) {
        tokio::spawn(async move {
            let should_start = {
                let mut reconnecting = ctx.reconnecting_servers.write().await;
                reconnecting.insert(server_name.clone())
            };
            if !should_start {
                return;
            }

            let delays_secs = [2_u64, 5, 10, 20, 30];
            let mut attempt = 0_usize;

            loop {
                if ctx.mcp_clients.read().await.contains_key(&server_name) {
                    break;
                }

                let server = match Self::load_mcp_server_for_reconnect(
                    ctx.store.as_ref(),
                    &ctx.owner_id,
                    &server_name,
                )
                .await
                {
                    Ok(Some(server)) if server.enabled => server,
                    Ok(Some(_)) | Ok(None) => {
                        Self::record_mcp_activity_with_store(
                            ctx.store.as_ref(),
                            &ctx.owner_id,
                            &server_name,
                            "connection",
                            "Stopped MCP reconnect because the server is disabled or removed",
                            None,
                        )
                        .await;
                        break;
                    }
                    Err(error) => {
                        Self::record_mcp_activity_with_store(
                            ctx.store.as_ref(),
                            &ctx.owner_id,
                            &server_name,
                            "error",
                            "Failed to load MCP server config for reconnect",
                            Some(error.to_string()),
                        )
                        .await;
                        break;
                    }
                };

                attempt += 1;
                match create_client_from_config(
                    server,
                    &ctx.mcp_session_manager,
                    &ctx.mcp_process_manager,
                    Some(Arc::clone(&ctx.secrets)),
                    &ctx.owner_id,
                )
                .await
                {
                    Ok(client) => {
                        match Self::attach_mcp_client_with_context(&server_name, client, &ctx).await
                        {
                            Ok(_) => {
                                Self::record_mcp_activity_with_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                    "connection",
                                    "Reconnected MCP client automatically",
                                    Some(format!("Recovered after {attempt} attempt(s)")),
                                )
                                .await;
                                break;
                            }
                            Err(error) => {
                                Self::record_mcp_activity_with_store(
                                    ctx.store.as_ref(),
                                    &ctx.owner_id,
                                    &server_name,
                                    "warning",
                                    "Automatic MCP reconnect attempt failed",
                                    Some(format!("Attempt {attempt}: {error}")),
                                )
                                .await;
                            }
                        }
                    }
                    Err(error) => {
                        Self::record_mcp_activity_with_store(
                            ctx.store.as_ref(),
                            &ctx.owner_id,
                            &server_name,
                            "warning",
                            "Failed to create MCP client during reconnect",
                            Some(format!("Attempt {attempt}: {error}")),
                        )
                        .await;
                    }
                }

                if attempt >= delays_secs.len() {
                    Self::record_mcp_activity_with_store(
                        ctx.store.as_ref(),
                        &ctx.owner_id,
                        &server_name,
                        "error",
                        "Stopped automatic MCP reconnect attempts",
                        Some(format!("Exceeded {} attempts", delays_secs.len())),
                    )
                    .await;
                    break;
                }

                tokio::time::sleep(std::time::Duration::from_secs(delays_secs[attempt - 1])).await;
            }

            ctx.reconnecting_servers.write().await.remove(&server_name);
        });
    }

    async fn unregister_mcp_tools_for_server(tool_registry: &Arc<ToolRegistry>, server_name: &str) {
        let prefix = format!("{server_name}_");
        let stale_tools: Vec<String> = tool_registry
            .list()
            .await
            .into_iter()
            .filter(|tool| tool.starts_with(&prefix))
            .collect();
        for tool in stale_tools {
            tool_registry.unregister(&tool).await;
        }
    }

    async fn fail_pending_requests_for_server(
        task_runtime: &Arc<RwLock<Option<Arc<TaskRuntime>>>>,
        pending_sampling_requests: &Arc<RwLock<HashMap<Uuid, PendingMcpSamplingRequest>>>,
        pending_elicitation_requests: &Arc<RwLock<HashMap<Uuid, PendingMcpElicitationRequest>>>,
        server_name: &str,
        reason: &str,
    ) -> usize {
        let sampling_task_ids: Vec<Uuid> = {
            let pending = pending_sampling_requests.read().await;
            pending
                .iter()
                .filter_map(|(task_id, pending)| {
                    (pending.server_name == server_name).then_some(*task_id)
                })
                .collect()
        };
        let elicitation_task_ids: Vec<Uuid> = {
            let pending = pending_elicitation_requests.read().await;
            pending
                .iter()
                .filter_map(|(task_id, pending)| {
                    (pending.server_name == server_name).then_some(*task_id)
                })
                .collect()
        };

        if let Some(runtime) = task_runtime.read().await.clone() {
            for task_id in &sampling_task_ids {
                let metadata = serde_json::json!({
                    "kind": "sampling",
                    "server_name": server_name,
                    "failure_reason": reason,
                });
                runtime
                    .mark_failed_with_result(*task_id, reason.to_string(), Some(metadata))
                    .await;
            }
            for task_id in &elicitation_task_ids {
                let metadata = serde_json::json!({
                    "kind": "elicitation",
                    "server_name": server_name,
                    "failure_reason": reason,
                });
                runtime
                    .mark_failed_with_result(*task_id, reason.to_string(), Some(metadata))
                    .await;
            }
        }

        if !sampling_task_ids.is_empty() {
            let ids: std::collections::HashSet<Uuid> = sampling_task_ids.iter().copied().collect();
            pending_sampling_requests
                .write()
                .await
                .retain(|task_id, _| !ids.contains(task_id));
        }
        if !elicitation_task_ids.is_empty() {
            let ids: std::collections::HashSet<Uuid> =
                elicitation_task_ids.iter().copied().collect();
            pending_elicitation_requests
                .write()
                .await
                .retain(|task_id, _| !ids.contains(task_id));
        }

        sampling_task_ids.len() + elicitation_task_ids.len()
    }

    async fn create_mcp_sampling_task(
        task_runtime: &Arc<RwLock<Option<Arc<TaskRuntime>>>>,
        server_name: &str,
        request: &McpSamplingRequest,
        mode: TaskMode,
    ) -> Option<Uuid> {
        let task_runtime = task_runtime.read().await.clone()?;
        let metadata = Self::sampling_task_metadata(server_name, request, None);
        let title = format!("MCP sampling request from {server_name}");
        let task = task_runtime
            .create_workflow_task("mcp:sampling", title, mode, Some(metadata.clone()))
            .await;
        if mode == TaskMode::Ask {
            let pending = TaskPendingApproval {
                id: task.id,
                risk: "model_sampling".to_string(),
                summary: "MCP server requested model sampling".to_string(),
                operations: vec![TaskOperation {
                    kind: "mcp_sampling".to_string(),
                    tool_name: server_name.to_string(),
                    parameters: metadata,
                    path: None,
                    destination_path: None,
                }],
                allow_always: false,
            };
            let _ = task_runtime.set_waiting_approval(task.id, pending).await;
        }
        Some(task.id)
    }

    async fn mcp_sampling_mode_for_request(
        task_runtime: &Arc<RwLock<Option<Arc<TaskRuntime>>>>,
        client: &Arc<McpClient>,
    ) -> TaskMode {
        let Some(conversation_id) = client.current_conversation_id().await else {
            return TaskMode::Ask;
        };
        let Some(task_runtime) = task_runtime.read().await.clone() else {
            return TaskMode::Ask;
        };
        task_runtime.mode_for_task(conversation_id).await
    }

    async fn create_mcp_elicitation_task(
        task_runtime: &Arc<RwLock<Option<Arc<TaskRuntime>>>>,
        server_name: &str,
        request: &McpElicitationRequest,
    ) -> Option<Uuid> {
        let task_runtime = task_runtime.read().await.clone()?;
        let metadata = Self::elicitation_task_metadata(server_name, request, None, None);
        let title = format!("MCP elicitation request from {server_name}");
        let task = task_runtime
            .create_workflow_task(
                "mcp:elicitation",
                title,
                TaskMode::Ask,
                Some(metadata.clone()),
            )
            .await;
        let pending = TaskPendingApproval {
            id: task.id,
            risk: "user_input".to_string(),
            summary: "MCP server requested elicitation".to_string(),
            operations: vec![TaskOperation {
                kind: "mcp_elicitation".to_string(),
                tool_name: server_name.to_string(),
                parameters: metadata,
                path: None,
                destination_path: None,
            }],
            allow_always: false,
        };
        let _ = task_runtime.set_waiting_approval(task.id, pending).await;
        Some(task.id)
    }

    fn sampling_task_metadata(
        server_name: &str,
        request: &McpSamplingRequest,
        preview: Option<&McpSamplingResult>,
    ) -> serde_json::Value {
        let mut value = serde_json::json!({
            "kind": "sampling",
            "server_name": server_name,
            "request": request,
        });
        if let Some(preview) = preview {
            value["preview"] = serde_json::to_value(preview).unwrap_or(serde_json::Value::Null);
        }
        value
    }

    fn elicitation_task_metadata(
        server_name: &str,
        request: &McpElicitationRequest,
        content: Option<&HashMap<String, serde_json::Value>>,
        action: Option<&str>,
    ) -> serde_json::Value {
        let mut value = serde_json::json!({
            "kind": "elicitation",
            "server_name": server_name,
            "request": request,
        });
        if let Some(content) = content {
            value["content"] = serde_json::to_value(content)
                .unwrap_or(serde_json::Value::Object(Default::default()));
        }
        if let Some(action) = action {
            value["action"] = serde_json::json!(action);
        }
        value
    }

    async fn generate_mcp_sampling_preview(
        &self,
        request: &McpSamplingRequest,
    ) -> Result<McpSamplingResult, ExtensionError> {
        Self::generate_mcp_sampling_preview_with_runtime(&self.runtime_llm, request).await
    }

    async fn generate_mcp_sampling_preview_with_runtime(
        runtime_llm: &Arc<RwLock<Option<Arc<dyn LlmProvider>>>>,
        request: &McpSamplingRequest,
    ) -> Result<McpSamplingResult, ExtensionError> {
        let llm = runtime_llm
            .read()
            .await
            .clone()
            .ok_or_else(|| ExtensionError::Other("LLM runtime not available".to_string()))?;

        let messages = Self::sampling_request_to_chat_messages(request)?;
        let mut completion = CompletionRequest::new(messages);
        if let Some(max_tokens) = request.max_tokens {
            completion = completion.with_max_tokens(max_tokens);
        }
        if let Some(temperature) = request.temperature {
            completion = completion.with_temperature(temperature);
        }
        if let Some(stop_sequences) = &request.stop_sequences {
            completion.stop_sequences = Some(stop_sequences.clone());
        }
        if let Some(model_preferences) = &request.model_preferences
            && let Some(first_hint) = model_preferences.hints.first()
        {
            completion = completion.with_model(first_hint.name.clone());
        }

        let response = llm
            .complete(completion)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        Ok(McpSamplingResult {
            role: "assistant".to_string(),
            content: McpSamplingContentBlock::Text {
                text: response.content,
                annotations: None,
            },
            model: Some(llm.effective_model_name(None)),
            stop_reason: Some(match response.finish_reason {
                crate::llm::FinishReason::Length => "maxTokens".to_string(),
                crate::llm::FinishReason::ToolUse => "toolUse".to_string(),
                crate::llm::FinishReason::ContentFilter => "contentFilter".to_string(),
                crate::llm::FinishReason::Stop | crate::llm::FinishReason::Unknown => {
                    "endTurn".to_string()
                }
            }),
        })
    }

    fn sampling_request_to_chat_messages(
        request: &McpSamplingRequest,
    ) -> Result<Vec<ChatMessage>, ExtensionError> {
        let mut messages = Vec::new();
        if let Some(system_prompt) = &request.system_prompt
            && !system_prompt.trim().is_empty()
        {
            messages.push(ChatMessage::system(system_prompt.clone()));
        }
        for message in &request.messages {
            messages.push(Self::sampling_message_to_chat_message(message)?);
        }
        Ok(messages)
    }

    fn sampling_message_to_chat_message(
        message: &crate::tools::mcp::McpSamplingMessage,
    ) -> Result<ChatMessage, ExtensionError> {
        match &message.content {
            McpSamplingContentBlock::Text { text, .. } => Ok(match message.role.as_str() {
                "assistant" => ChatMessage::assistant(text.clone()),
                "system" => ChatMessage::system(text.clone()),
                _ => ChatMessage::user(text.clone()),
            }),
            McpSamplingContentBlock::Image {
                data, mime_type, ..
            } => {
                let text = format!("[image input: {mime_type}]");
                let parts = vec![ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: format!("data:{mime_type};base64,{data}"),
                        detail: Some("auto".to_string()),
                    },
                }];
                Ok(match message.role.as_str() {
                    "assistant" => ChatMessage {
                        role: Role::Assistant,
                        content: text,
                        content_parts: parts,
                        tool_call_id: None,
                        name: None,
                        tool_calls: None,
                    },
                    "system" => ChatMessage {
                        role: Role::System,
                        content: text,
                        content_parts: parts,
                        tool_call_id: None,
                        name: None,
                        tool_calls: None,
                    },
                    _ => ChatMessage::user_with_parts(text, parts),
                })
            }
            McpSamplingContentBlock::Audio { .. } => Err(ExtensionError::ActivationFailed(
                "Current Steward provider does not support audio sampling requests".to_string(),
            )),
        }
    }

    fn validate_mcp_elicitation_content(
        request: &McpElicitationRequest,
        content: &HashMap<String, serde_json::Value>,
    ) -> Result<(), ExtensionError> {
        for required in &request.requested_schema.required {
            if !content.contains_key(required) {
                return Err(ExtensionError::ActivationFailed(format!(
                    "Missing required elicitation field '{}'",
                    required
                )));
            }
        }

        for (name, value) in content {
            let schema = request
                .requested_schema
                .properties
                .get(name)
                .ok_or_else(|| {
                    ExtensionError::ActivationFailed(format!(
                        "Unexpected elicitation field '{}'",
                        name
                    ))
                })?;
            Self::validate_primitive_schema_value(name, schema, value)?;
        }

        Ok(())
    }

    fn validate_primitive_schema_value(
        field_name: &str,
        schema: &McpPrimitiveSchemaDefinition,
        value: &serde_json::Value,
    ) -> Result<(), ExtensionError> {
        match schema {
            McpPrimitiveSchemaDefinition::String {
                enum_values,
                min_length,
                max_length,
                ..
            } => {
                let Some(text) = value.as_str() else {
                    return Err(ExtensionError::ActivationFailed(format!(
                        "Field '{}' must be a string",
                        field_name
                    )));
                };
                if let Some(enum_values) = enum_values
                    && !enum_values.iter().any(|candidate| candidate == text)
                {
                    return Err(ExtensionError::ActivationFailed(format!(
                        "Field '{}' must be one of: {}",
                        field_name,
                        enum_values.join(", ")
                    )));
                }
                if let Some(min_length) = min_length
                    && text.chars().count() < *min_length as usize
                {
                    return Err(ExtensionError::ActivationFailed(format!(
                        "Field '{}' must be at least {} characters",
                        field_name, min_length
                    )));
                }
                if let Some(max_length) = max_length
                    && text.chars().count() > *max_length as usize
                {
                    return Err(ExtensionError::ActivationFailed(format!(
                        "Field '{}' must be at most {} characters",
                        field_name, max_length
                    )));
                }
            }
            McpPrimitiveSchemaDefinition::Number {
                minimum, maximum, ..
            } => {
                let Some(number) = value.as_f64() else {
                    return Err(ExtensionError::ActivationFailed(format!(
                        "Field '{}' must be a number",
                        field_name
                    )));
                };
                if let Some(minimum) = minimum
                    && number < *minimum
                {
                    return Err(ExtensionError::ActivationFailed(format!(
                        "Field '{}' must be >= {}",
                        field_name, minimum
                    )));
                }
                if let Some(maximum) = maximum
                    && number > *maximum
                {
                    return Err(ExtensionError::ActivationFailed(format!(
                        "Field '{}' must be <= {}",
                        field_name, maximum
                    )));
                }
            }
            McpPrimitiveSchemaDefinition::Integer {
                minimum, maximum, ..
            } => {
                let Some(number) = value.as_i64() else {
                    return Err(ExtensionError::ActivationFailed(format!(
                        "Field '{}' must be an integer",
                        field_name
                    )));
                };
                if let Some(minimum) = minimum
                    && number < *minimum
                {
                    return Err(ExtensionError::ActivationFailed(format!(
                        "Field '{}' must be >= {}",
                        field_name, minimum
                    )));
                }
                if let Some(maximum) = maximum
                    && number > *maximum
                {
                    return Err(ExtensionError::ActivationFailed(format!(
                        "Field '{}' must be <= {}",
                        field_name, maximum
                    )));
                }
            }
            McpPrimitiveSchemaDefinition::Boolean { .. } => {
                if !value.is_boolean() {
                    return Err(ExtensionError::ActivationFailed(format!(
                        "Field '{}' must be a boolean",
                        field_name
                    )));
                }
            }
        }
        Ok(())
    }

    async fn finish_pending_sampling(
        &self,
        task_id: Uuid,
        pending: &PendingMcpSamplingRequest,
        payload: serde_json::Value,
        task_metadata: serde_json::Value,
    ) -> Result<crate::task_runtime::TaskRecord, ExtensionError> {
        self.pending_sampling_requests
            .write()
            .await
            .remove(&task_id);
        let client = self
            .mcp_clients
            .read()
            .await
            .get(&pending.server_name)
            .cloned()
            .ok_or_else(|| ExtensionError::ActivationFailed("MCP client not found".to_string()))?;
        client
            .respond_success(pending.request_id.clone(), payload)
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        let task_runtime = self
            .task_runtime
            .read()
            .await
            .clone()
            .ok_or_else(|| ExtensionError::Other("Task runtime not available".to_string()))?;
        task_runtime
            .mark_completed_with_result(task_id, Some(task_metadata))
            .await;
        task_runtime
            .get_task(task_id)
            .await
            .ok_or_else(|| ExtensionError::ActivationFailed("MCP task not found".to_string()))
    }

    async fn reject_pending_sampling(
        &self,
        task_id: Uuid,
        pending: &PendingMcpSamplingRequest,
        cancelled: bool,
    ) -> Result<crate::task_runtime::TaskRecord, ExtensionError> {
        self.pending_sampling_requests
            .write()
            .await
            .remove(&task_id);
        let client = self
            .mcp_clients
            .read()
            .await
            .get(&pending.server_name)
            .cloned()
            .ok_or_else(|| ExtensionError::ActivationFailed("MCP client not found".to_string()))?;
        client
            .respond_error(
                pending.request_id.clone(),
                -32800,
                if cancelled {
                    "User cancelled MCP sampling request"
                } else {
                    "User declined MCP sampling request"
                },
                None,
            )
            .await
            .map_err(|e| ExtensionError::ActivationFailed(e.to_string()))?;

        let task_runtime = self
            .task_runtime
            .read()
            .await
            .clone()
            .ok_or_else(|| ExtensionError::Other("Task runtime not available".to_string()))?;
        let metadata = Self::sampling_task_metadata(&pending.server_name, &pending.request, None);
        if cancelled {
            task_runtime
                .mark_cancelled(task_id, "Cancelled from MCP panel")
                .await;
        } else {
            task_runtime
                .mark_rejected(task_id, "Declined from MCP panel")
                .await;
        }
        task_runtime
            .update_result_metadata(task_id, metadata)
            .await
            .ok_or_else(|| ExtensionError::ActivationFailed("MCP task not found".to_string()))
    }

    async fn ensure_mcp_client(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<Arc<McpClient>, ExtensionError> {
        if let Some(client) = self.mcp_clients.read().await.get(name).cloned() {
            match client.ping().await {
                Ok(()) => return Ok(client),
                Err(error) => {
                    tracing::warn!(
                        server = %name,
                        %error,
                        "Cached MCP client failed health probe; rebuilding connection"
                    );
                    {
                        let mut clients = self.mcp_clients.write().await;
                        if let Some(active_client) = clients.get(name)
                            && Arc::ptr_eq(active_client, &client)
                        {
                            clients.remove(name);
                        }
                    }
                    Self::unregister_mcp_tools_for_server(&self.tool_registry, name).await;
                    Self::record_mcp_activity_with_store(
                        self.store.as_ref(),
                        &self.user_id,
                        name,
                        "connection",
                        "Dropped stale MCP client; reconnecting on demand",
                        Some(error.to_string()),
                    )
                    .await;
                }
            }
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
        self.cache_and_attach_mcp_client(name, client).await
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

    fn sanitize_snapshot_segment(value: &str) -> String {
        let sanitized: String = value
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect();
        let trimmed = sanitized
            .trim_matches('-')
            .chars()
            .take(64)
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        if trimmed.is_empty() {
            "snapshot".to_string()
        } else {
            trimmed
        }
    }

    fn extension_for_mime(mime_type: Option<&str>) -> Option<&'static str> {
        match mime_type.unwrap_or_default() {
            "text/plain" => Some(".txt"),
            "text/markdown" => Some(".md"),
            "application/json" => Some(".json"),
            "text/html" => Some(".html"),
            "image/png" => Some(".png"),
            "image/jpeg" => Some(".jpg"),
            "image/webp" => Some(".webp"),
            "application/pdf" => Some(".pdf"),
            _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fail_pending_requests_for_server_marks_matching_tasks_failed_and_clears_maps() {
        let task_runtime = Arc::new(TaskRuntime::new());
        let sampling_task = task_runtime
            .create_workflow_task("mcp:sampling", "sampling", TaskMode::Ask, None)
            .await;
        let elicitation_task = task_runtime
            .create_workflow_task("mcp:elicitation", "elicitation", TaskMode::Ask, None)
            .await;
        let other_task = task_runtime
            .create_workflow_task("mcp:sampling", "other", TaskMode::Ask, None)
            .await;

        let task_runtime_slot = Arc::new(RwLock::new(Some(Arc::clone(&task_runtime))));
        let pending_sampling_requests = Arc::new(RwLock::new(HashMap::from([
            (
                sampling_task.id,
                PendingMcpSamplingRequest {
                    server_name: "demo".to_string(),
                    request_id: serde_json::json!("sampling-1"),
                    request: McpSamplingRequest {
                        messages: Vec::new(),
                        system_prompt: None,
                        model_preferences: None,
                        max_tokens: None,
                        temperature: None,
                        stop_sequences: None,
                        include_context: None,
                        metadata: None,
                    },
                },
            ),
            (
                other_task.id,
                PendingMcpSamplingRequest {
                    server_name: "other".to_string(),
                    request_id: serde_json::json!("sampling-2"),
                    request: McpSamplingRequest {
                        messages: Vec::new(),
                        system_prompt: None,
                        model_preferences: None,
                        max_tokens: None,
                        temperature: None,
                        stop_sequences: None,
                        include_context: None,
                        metadata: None,
                    },
                },
            ),
        ])));
        let pending_elicitation_requests = Arc::new(RwLock::new(HashMap::from([(
            elicitation_task.id,
            PendingMcpElicitationRequest {
                server_name: "demo".to_string(),
                request_id: serde_json::json!("elicitation-1"),
                request: McpElicitationRequest {
                    message: "Need input".to_string(),
                    requested_schema: crate::tools::mcp::McpElicitationSchema {
                        schema_type: "object".to_string(),
                        properties: HashMap::new(),
                        required: Vec::new(),
                    },
                },
            },
        )])));

        let failed = ExtensionManager::fail_pending_requests_for_server(
            &task_runtime_slot,
            &pending_sampling_requests,
            &pending_elicitation_requests,
            "demo",
            "connection closed",
        )
        .await;

        assert_eq!(failed, 2);
        assert!(
            !pending_sampling_requests
                .read()
                .await
                .contains_key(&sampling_task.id)
        );
        assert!(
            !pending_elicitation_requests
                .read()
                .await
                .contains_key(&elicitation_task.id)
        );
        assert!(
            pending_sampling_requests
                .read()
                .await
                .contains_key(&other_task.id)
        );

        let sampling_status = task_runtime
            .get_task(sampling_task.id)
            .await
            .unwrap()
            .status;
        let elicitation_status = task_runtime
            .get_task(elicitation_task.id)
            .await
            .unwrap()
            .status;
        let other_status = task_runtime.get_task(other_task.id).await.unwrap().status;

        assert_eq!(sampling_status, crate::task_runtime::TaskStatus::Failed);
        assert_eq!(elicitation_status, crate::task_runtime::TaskStatus::Failed);
        assert_eq!(other_status, crate::task_runtime::TaskStatus::Queued);
    }
}
