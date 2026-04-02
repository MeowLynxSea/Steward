use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;

use crate::agent::{Agent, AgentDeps};
use crate::api::{ApiState, local_api_addr, run_api};
use crate::app::{AppBuilder, AppBuilderFlags};
use crate::channels::ChannelManager;
use crate::config::Config;
use crate::hooks::bootstrap_hooks;
use crate::llm::{
    ReloadableLlmProvider, ReloadableLlmState, ReloadableSlot, RuntimeLlmReloader,
    create_session_manager,
};
use crate::orchestrator::{ReaperConfig, SandboxReaper};
use crate::runtime_events::SseManager;
use crate::task_runtime::TaskRuntime;
use crate::tracing_fmt::init_app_tracing;

pub async fn start_embedded_runtime(api_port: u16) -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    crate::bootstrap::load_ironclaw_env();

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
    let orch = crate::orchestrator::setup_orchestrator(
        &config,
        &components.llm,
        components.db.as_ref(),
        components.secrets_store.as_ref(),
    )
    .await;
    let container_job_manager = orch.container_job_manager;
    let job_event_tx = orch.job_event_tx;
    let prompt_queue = orch.prompt_queue;
    let docker_status = orch.docker_status;

    let channels = Arc::new(ChannelManager::new());

    let active_tool_names = components.tools.list().await;
    let _ = bootstrap_hooks(
        &components.hooks,
        components.workspace.as_ref(),
        &config.wasm.tools_dir,
        &config.channels.wasm_channels_dir,
        &active_tool_names,
        &[],
        &components.dev_loaded_tool_names,
    )
    .await;

    let session_manager = Arc::clone(&components.agent_session_manager);
    let scheduler_slot: crate::tools::builtin::SchedulerSlot =
        Arc::new(tokio::sync::RwLock::new(None));

    components.tools.register_job_tools(
        Arc::clone(&components.context_manager),
        Some(scheduler_slot.clone()),
        container_job_manager.clone(),
        components.db.clone(),
        job_event_tx.clone(),
        Some(channels.inject_sender()),
        if config.sandbox.enabled {
            Some(Arc::clone(&prompt_queue))
        } else {
            None
        },
        components.secrets_store.clone(),
    );

    components
        .tools
        .register_message_tools(Arc::clone(&channels), components.extension_manager.clone())
        .await;

    let reaper_context_manager = Arc::clone(&components.context_manager);
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
    let runtime_llm_reloader = Arc::new(RuntimeLlmReloader::new(
        Arc::clone(&reloadable_llm_state),
        components.session.clone(),
        config.owner_id.clone(),
        components.secrets_store.clone(),
    ));
    let app_llm: Arc<dyn crate::llm::LlmProvider> = Arc::new(ReloadableLlmProvider::new(
        Arc::clone(&reloadable_llm_state),
        ReloadableSlot::Primary,
    ));
    let app_cheap_llm: Arc<dyn crate::llm::LlmProvider> = Arc::new(ReloadableLlmProvider::new(
        Arc::clone(&reloadable_llm_state),
        ReloadableSlot::Cheap,
    ));

    if let Some(store) = components.db.clone() {
        let api_bind_addr = local_api_addr(api_port);
        let mut api_state = ApiState::new(
            config.owner_id.clone(),
            api_bind_addr,
            store,
            sse_manager.clone(),
            Some(task_runtime.clone()),
            Some(channels.inject_sender()),
            Some(session_manager.clone()),
            components.workspace.clone(),
        )
        .with_llm_reloader(runtime_llm_reloader)
        .with_workbench_metadata(
            components.tools.count(),
            components.dev_loaded_tool_names.clone(),
        );
        if let Some(secrets_store) = components.secrets_store.clone() {
            api_state = api_state.with_secrets_store(secrets_store);
        }
        tokio::spawn(async move {
            if let Err(error) = run_api(api_bind_addr, api_state).await {
                tracing::error!(%error, "embedded local api service exited");
            }
        });
    }

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
        http_interceptor: components
            .recording_handle
            .as_ref()
            .map(|recording| recording.http_interceptor()),
        transcription: config.transcription.create_provider().map(|provider| {
            Arc::new(crate::llm::transcription::TranscriptionMiddleware::new(provider))
        }),
        document_extraction: Some(Arc::new(
            crate::document_extraction::DocumentExtractionMiddleware::new(),
        )),
        sandbox_readiness: if !config.sandbox.enabled {
            crate::agent::routine_engine::SandboxReadiness::DisabledByConfig
        } else if docker_status.is_ok() {
            crate::agent::routine_engine::SandboxReadiness::Available
        } else {
            crate::agent::routine_engine::SandboxReadiness::DockerUnavailable
        },
        builder: components.builder,
        llm_backend: config.llm.backend.clone(),
        tenant_rates: Arc::new(crate::tenant::TenantRateRegistry::new(
            config.agent.max_llm_concurrent_per_user.unwrap_or(4),
            config.agent.max_jobs_concurrent_per_user.unwrap_or(3),
        )),
        task_runtime: Some(task_runtime),
    };

    let routine_engine_slot = Arc::new(tokio::sync::RwLock::new(None));
    let mut agent = Agent::new(
        config.agent.clone(),
        deps,
        channels,
        Some(config.heartbeat.clone()),
        Some(config.hygiene.clone()),
        Some(config.routines.clone()),
        Some(components.context_manager),
        Some(session_manager),
    );
    *scheduler_slot.write().await = Some(agent.scheduler());
    agent.set_routine_engine_slot(Arc::clone(&routine_engine_slot));

    if let Some(ref jm) = container_job_manager {
        let reaper_jm = Arc::clone(jm);
        let reaper_config = ReaperConfig {
            scan_interval: Duration::from_secs(config.sandbox.reaper_interval_secs),
            orphan_threshold: Duration::from_secs(config.sandbox.orphan_threshold_secs),
            ..ReaperConfig::default()
        };
        let reaper_ctx = Arc::clone(&reaper_context_manager);
        tokio::spawn(async move {
            match SandboxReaper::new(reaper_jm, reaper_ctx, reaper_config).await {
                Ok(reaper) => reaper.run().await,
                Err(error) => tracing::error!("Sandbox reaper failed to initialize: {}", error),
            }
        });
    }

    tokio::spawn(async move {
        if let Err(error) = agent.run().await {
            tracing::error!(%error, "embedded agent exited");
        }
    });

    wait_for_api_ready(api_port).await?;
    Ok(())
}

async fn wait_for_api_ready(api_port: u16) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .context("failed to create embedded runtime readiness probe client")?;
    let health_url = format!("http://127.0.0.1:{api_port}/api/v0/health");

    for _ in 0..120 {
        if let Ok(response) = client.get(&health_url).send().await
            && response.status().is_success()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    anyhow::bail!("embedded local api failed to become ready on {health_url}")
}
