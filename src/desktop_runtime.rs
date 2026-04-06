use std::sync::Arc;

use crate::agent::SessionManager as AgentSessionManager;
use crate::agent::{Agent, AgentDeps};
use crate::app::{AppBuilder, AppBuilderFlags};
use crate::channels::{ChannelManager, MessageStream};
use crate::config::Config;
use crate::db::Database;
use crate::hooks::bootstrap_hooks;
use crate::llm::{
    ReloadableLlmProvider, ReloadableLlmState, ReloadableSlot, create_session_manager,
};
use crate::runtime_events::{RuntimeEventEmitter, SseManager};
use crate::task_runtime::TaskRuntime;
use crate::tools::mcp::McpSessionManager;
use crate::tracing_fmt::init_app_tracing;
use crate::workspace::Workspace;

/// Optional Tauri event emitter for native desktop events.
/// When provided, events will be emitted via Tauri in addition to SSE.
pub type TauriEventEmitterHandle = Arc<dyn RuntimeEventEmitter>;

/// Application state shared with the Tauri IPC layer.
/// Created during desktop runtime startup and passed to IPC commands.
pub struct AppState {
    pub owner_id: String,
    pub db: Option<Arc<dyn Database>>,
    pub workspace: Option<Arc<Workspace>>,
    pub agent_session_manager: Arc<AgentSessionManager>,
    pub task_runtime: Arc<TaskRuntime>,
    pub tools: Arc<crate::tools::ToolRegistry>,
    pub mcp_session_manager: Arc<McpSessionManager>,
    /// Sender to inject messages into the agent's message stream.
    /// Used by Tauri IPC commands to trigger agent processing.
    pub message_inject_tx: tokio::sync::mpsc::Sender<crate::channels::IncomingMessage>,
}

impl AppState {
    pub fn new(
        owner_id: String,
        db: Option<Arc<dyn Database>>,
        workspace: Option<Arc<Workspace>>,
        agent_session_manager: Arc<AgentSessionManager>,
        task_runtime: Arc<TaskRuntime>,
        tools: Arc<crate::tools::ToolRegistry>,
        mcp_session_manager: Arc<McpSessionManager>,
        message_inject_tx: tokio::sync::mpsc::Sender<crate::channels::IncomingMessage>,
    ) -> Self {
        Self {
            owner_id,
            db,
            workspace,
            agent_session_manager,
            task_runtime,
            tools,
            mcp_session_manager,
            message_inject_tx,
        }
    }
}

pub async fn start_embedded_runtime(
    tauri_emitter: Option<TauriEventEmitterHandle>,
) -> anyhow::Result<AppState> {
    let _ = dotenvy::dotenv();
    crate::bootstrap::load_steward_env();

    let config = Config::from_env().await?;
    let session = create_session_manager(config.llm.session.clone()).await;

    init_app_tracing();

    let components = AppBuilder::new(
        config,
        AppBuilderFlags { no_db: false },
        None,
        session.clone(),
    )
    .build_all()
    .await?;

    let config = components.config;
    let active_tool_names = components.tools.list().await;
    let _ = bootstrap_hooks(
        &components.hooks,
        components.workspace.as_ref(),
        &config.wasm.tools_dir,
        &active_tool_names,
        &components.dev_loaded_tool_names,
    )
    .await;

    let session_manager = Arc::clone(&components.agent_session_manager);
    let channel_manager = Arc::new(ChannelManager::new());
    let inject_tx = channel_manager.inject_sender();
    let message_stream: MessageStream = channel_manager
        .start_all()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let scheduler_slot: crate::tools::builtin::SchedulerSlot =
        Arc::new(tokio::sync::RwLock::new(None));

    components.tools.register_job_tools(
        Arc::clone(&components.context_manager),
        Some(scheduler_slot.clone()),
        components.db.clone(),
        Some(inject_tx.clone()),
        components.secrets_store.clone(),
    );

    let task_runtime = if let Some(store) = components.db.clone() {
        Arc::new(TaskRuntime::with_store(config.owner_id.clone(), store))
    } else {
        Arc::new(TaskRuntime::new())
    };
    let sse_manager = Arc::new(SseManager::new());
    let primary_llm = components.llm.clone();
    let cheap_llm = components
        .cheap_llm
        .clone()
        .unwrap_or_else(|| primary_llm.clone());
    let reloadable_llm_state = Arc::new(ReloadableLlmState::new(primary_llm, cheap_llm));
    let app_llm: Arc<dyn crate::llm::LlmProvider> = Arc::new(ReloadableLlmProvider::new(
        Arc::clone(&reloadable_llm_state),
        ReloadableSlot::Primary,
    ));
    let app_cheap_llm: Arc<dyn crate::llm::LlmProvider> = Arc::new(ReloadableLlmProvider::new(
        Arc::clone(&reloadable_llm_state),
        ReloadableSlot::Cheap,
    ));

    // Clone values needed for AppState BEFORE moving components into AgentDeps
    let app_state_db = components.db.clone();
    let app_state_workspace = components.workspace.clone();
    let app_state_tools = Arc::clone(&components.tools);
    let app_state_mcp = Arc::clone(&components.mcp_session_manager);
    let app_state_session_manager = Arc::clone(&session_manager);
    let app_state_task_runtime = Arc::clone(&task_runtime);
    let extension_manager = components.extension_manager.clone();

    if let Some(extension_manager) = extension_manager.as_ref()
        && config.channels.wasm_channels.enabled
    {
        let runtime = Arc::new(crate::channels::wasm::WasmChannelRuntime::new(
            crate::channels::wasm::WasmChannelRuntimeConfig::default(),
        )?);
        extension_manager
            .set_channel_runtime(Arc::clone(&channel_manager), Arc::clone(&runtime))
            .await;

        let active_channels = extension_manager
            .load_persisted_active_channels(&config.owner_id)
            .await;
        for channel_name in active_channels {
            if let Err(error) = extension_manager
                .activate(&channel_name, &config.owner_id)
                .await
            {
                tracing::warn!(channel = %channel_name, %error, "Failed to restore wasm channel");
            }
        }
    }

    let deps = AgentDeps {
        owner_id: config.owner_id.clone(),
        store: components.db,
        llm: app_llm,
        cheap_llm: Some(app_cheap_llm),
        safety: components.safety,
        tools: components.tools,
        workspace: components.workspace,
        extension_manager,
        skill_registry: components.skill_registry,
        skill_catalog: components.skill_catalog,
        skills_config: config.skills.clone(),
        hooks: components.hooks,
        cost_guard: components.cost_guard,
        sse_tx: Some(sse_manager),
        emitter: tauri_emitter.clone(),
        http_interceptor: components
            .recording_handle
            .as_ref()
            .map(|recording| recording.http_interceptor()),
        transcription: config.transcription.create_provider().map(|provider| {
            Arc::new(crate::llm::transcription::TranscriptionMiddleware::new(
                provider,
            ))
        }),
        document_extraction: Some(Arc::new(
            crate::document_extraction::DocumentExtractionMiddleware::new(),
        )),
        claude_code_config: config.claude_code.clone(),
        builder: components.builder,
        llm_backend: config.llm.backend.clone(),
        tenant_rates: Arc::new(crate::tenant::TenantRateRegistry::new(
            config.agent.max_llm_concurrent_per_user.unwrap_or(4),
            config.agent.max_jobs_concurrent_per_user.unwrap_or(3),
        )),
        task_runtime: Some(task_runtime),
    };

    let routine_engine_slot = Arc::new(tokio::sync::RwLock::new(None));
    let mut agent = Agent::new_with_message_stream(
        config.agent.clone(),
        deps,
        message_stream,
        Some(channel_manager),
        Some(config.heartbeat.clone()),
        Some(config.hygiene.clone()),
        Some(config.routines.clone()),
        Some(components.context_manager),
        Some(session_manager),
    );
    *scheduler_slot.write().await = Some(agent.scheduler());
    agent.set_routine_engine_slot(Arc::clone(&routine_engine_slot));

    tracing::info!("Starting embedded agent runtime...");
    tokio::spawn(async move {
        tracing::info!("Embedded agent task started");
        if let Err(error) = agent.run().await {
            tracing::error!(%error, "embedded agent exited");
        }
    });

    Ok(AppState::new(
        config.owner_id.clone(),
        app_state_db,
        app_state_workspace,
        app_state_session_manager,
        app_state_task_runtime,
        app_state_tools,
        app_state_mcp,
        inject_tx.clone(),
    ))
}
