use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::agent::{Agent, AgentDeps};
use crate::app::{AppBuilder, AppBuilderFlags};
use crate::channels::{IncomingMessage, MessageStream};
use crate::config::Config;
use crate::hooks::bootstrap_hooks;
use crate::llm::{
    ReloadableLlmProvider, ReloadableLlmState, ReloadableSlot,
    create_session_manager,
};
use crate::runtime_events::{RuntimeEventEmitter, SseManager};
use crate::task_runtime::TaskRuntime;
use crate::tracing_fmt::init_app_tracing;
use crate::agent::SessionManager as AgentSessionManager;
use crate::tools::mcp::McpSessionManager;
use crate::workspace::Workspace;
use crate::db::Database;

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
    ) -> Self {
        Self {
            owner_id,
            db,
            workspace,
            agent_session_manager,
            task_runtime,
            tools,
            mcp_session_manager,
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
    let (inject_tx, inject_rx) = mpsc::channel::<IncomingMessage>(64);
    let message_stream: MessageStream = Box::pin(ReceiverStream::new(inject_rx));
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

    let deps = AgentDeps {
        owner_id: config.owner_id.clone(),
        store: components.db,
        llm: app_llm,
        cheap_llm: Some(app_cheap_llm),
        safety: components.safety,
        tools: components.tools,
        workspace: components.workspace,
        extension_manager: components.extension_manager,
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
        None,
        Some(config.heartbeat.clone()),
        Some(config.hygiene.clone()),
        Some(config.routines.clone()),
        Some(components.context_manager),
        Some(session_manager),
    );
    *scheduler_slot.write().await = Some(agent.scheduler());
    agent.set_routine_engine_slot(Arc::clone(&routine_engine_slot));

    tokio::spawn(async move {
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
    ))
}
