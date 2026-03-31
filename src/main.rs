//! IronClaw - Main entry point.

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;

use ironclaw::{
    agent::{Agent, AgentDeps},
    app::{AppBuilder, AppBuilderFlags},
    channels::{ChannelManager, ReplChannel},
    cli::{
        Cli, Command, run_api_command, run_mcp_command, run_pairing_command, run_service_command,
        run_status_command, run_tool_command,
    },
    config::Config,
    hooks::bootstrap_hooks,
    llm::create_session_manager,
    orchestrator::{ReaperConfig, SandboxReaper},
    tracing_fmt::{init_app_tracing, init_cli_tracing, init_worker_tracing},
};
/// Synchronous entry point. Loads `.env` files before the Tokio runtime
/// starts so that `std::env::set_var` is safe (no worker threads yet).
fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    ironclaw::bootstrap::load_ironclaw_env();

    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main());

    if let Err(ref e) = result {
        format_top_level_error(e);
    }
    result
}

/// Format a top-level error with color and recovery hints.
fn format_top_level_error(err: &anyhow::Error) {
    use ironclaw::cli::fmt;
    let msg = format!("{err:#}");

    eprintln!();
    eprintln!("  {}\u{2717}{} {}", fmt::error(), fmt::reset(), msg);

    // Provide recovery hints for common errors
    let lower = msg.to_ascii_lowercase();
    let hint = if lower.contains("database_url")
        || lower.contains("database") && lower.contains("not set")
    {
        Some("set LIBSQL_PATH or let the default local database path be created automatically")
    } else if lower.contains("connection refused") || lower.contains("connect error") {
        Some("check local service settings or libSQL file permissions")
    } else if lower.contains("session") && lower.contains("not found") {
        Some("set LLM_BACKEND and provider credentials in your environment or config")
    } else if lower.contains("secrets_master_key") {
        Some("set SECRETS_MASTER_KEY in .env or rely on the OS keychain")
    } else if lower.contains("already running") {
        Some("stop the other instance or remove the stale PID file")
    } else if lower.contains("onboard") {
        Some(
            "the interactive onboarding flow is deprecated; configure env vars or config.toml directly",
        )
    } else {
        None
    };

    if let Some(hint_text) = hint {
        eprintln!("  {}hint:{} {}", fmt::dim(), fmt::reset(), hint_text,);
    }
    eprintln!();
}

async fn async_main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Handle non-agent commands first (they don't need full setup)
    match &cli.command {
        Some(Command::Tool(tool_cmd)) => {
            init_cli_tracing();
            return run_tool_command(tool_cmd.clone()).await;
        }
        Some(Command::Config(config_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_config_command(config_cmd.clone()).await;
        }
        Some(Command::Api(api_cmd)) => {
            init_cli_tracing();
            return run_api_command(api_cmd, cli.config.as_deref()).await;
        }
        Some(Command::Registry(registry_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_registry_command(registry_cmd.clone()).await;
        }
        Some(Command::Routines(routines_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_routines_cli(routines_cmd, cli.config.as_deref()).await;
        }
        Some(Command::Mcp(mcp_cmd)) => {
            init_cli_tracing();
            return run_mcp_command(*mcp_cmd.clone()).await;
        }
        Some(Command::Memory(mem_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_memory_command(mem_cmd).await;
        }
        Some(Command::Pairing(pairing_cmd)) => {
            init_cli_tracing();
            return run_pairing_command(pairing_cmd.clone()).map_err(|e| anyhow::anyhow!("{}", e));
        }
        Some(Command::Service(service_cmd)) => {
            init_cli_tracing();
            return run_service_command(service_cmd);
        }
        Some(Command::Skills(skills_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_skills_command(skills_cmd.clone(), cli.config.as_deref())
                .await;
        }
        Some(Command::Hooks(hooks_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_hooks_command(hooks_cmd.clone(), cli.config.as_deref())
                .await;
        }
        Some(Command::Models(models_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_models_command(models_cmd.clone(), cli.config.as_deref())
                .await;
        }
        Some(Command::Doctor) => {
            init_cli_tracing();
            return ironclaw::cli::run_doctor_command().await;
        }
        Some(Command::Status) => {
            init_cli_tracing();
            return run_status_command().await;
        }
        Some(Command::Completion(completion)) => {
            init_cli_tracing();
            return completion.run();
        }
        #[cfg(feature = "import")]
        Some(Command::Import(import_cmd)) => {
            init_cli_tracing();
            let config = ironclaw::config::Config::from_env().await?;
            return ironclaw::cli::run_import_command(import_cmd, &config).await;
        }
        Some(Command::Worker {
            job_id,
            orchestrator_url,
            max_iterations,
        }) => {
            init_worker_tracing();
            return ironclaw::worker::run_worker(*job_id, orchestrator_url, *max_iterations).await;
        }
        Some(Command::ClaudeBridge {
            job_id,
            orchestrator_url,
            max_turns,
            model,
        }) => {
            init_worker_tracing();
            return ironclaw::worker::run_claude_bridge(
                *job_id,
                orchestrator_url,
                *max_turns,
                model,
            )
            .await;
        }
        Some(Command::Login { openai_codex }) => {
            init_cli_tracing();
            if *openai_codex {
                // Resolve codex config so OPENAI_CODEX_* env overrides are
                // honoured even when LLM_BACKEND isn't set to openai_codex.
                let codex_config = {
                    let config = Config::from_env()
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?;
                    config.llm.openai_codex.unwrap_or_else(|| {
                        use ironclaw::llm::OpenAiCodexConfig;
                        let mut cfg = OpenAiCodexConfig::default();
                        if let Ok(v) = std::env::var("OPENAI_CODEX_AUTH_URL") {
                            cfg.auth_endpoint = v;
                        }
                        if let Ok(v) = std::env::var("OPENAI_CODEX_API_URL") {
                            cfg.api_base_url = v;
                        }
                        if let Ok(v) = std::env::var("OPENAI_CODEX_CLIENT_ID") {
                            cfg.client_id = v;
                        }
                        if let Ok(v) = std::env::var("OPENAI_CODEX_SESSION_PATH") {
                            cfg.session_path = std::path::PathBuf::from(v);
                        }
                        cfg
                    })
                };
                let mgr = ironclaw::llm::OpenAiCodexSessionManager::new(codex_config)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                mgr.device_code_login()
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                println!(
                    "OpenAI Codex authentication complete. Set LLM_BACKEND=openai_codex to use it."
                );
            } else {
                println!("Specify a provider to authenticate with:");
                println!("  ironclaw login --openai-codex   (ChatGPT subscription)");
            }
            return Ok(());
        }
        None | Some(Command::Run) => {
            // Continue to run agent
        }
    }

    // ── PID lock (prevent multiple instances) ────────────────────────
    let _pid_lock = match ironclaw::bootstrap::PidLock::acquire() {
        Ok(lock) => Some(lock),
        Err(ironclaw::bootstrap::PidLockError::AlreadyRunning { pid }) => {
            anyhow::bail!(
                "Another IronClaw instance is already running (PID {}). \
                 If this is incorrect, remove the stale PID file: {}",
                pid,
                ironclaw::bootstrap::pid_lock_path().display()
            );
        }
        Err(e) => {
            eprintln!("Warning: Could not acquire PID lock: {}", e);
            eprintln!("Continuing without PID lock protection.");
            None
        }
    };

    let startup_start = std::time::Instant::now();

    // ── Agent startup ──────────────────────────────────────────────────

    // Load initial config from env + disk + optional TOML (before DB is available).
    // Credentials may be missing at this point — that's fine. LlmConfig::resolve()
    // defers gracefully, and AppBuilder::build_all() re-resolves after loading
    // secrets from the encrypted DB.
    let toml_path = cli.config.as_deref();
    let config = match Config::from_env_with_toml(toml_path).await {
        Ok(c) => c,
        Err(ironclaw::error::ConfigError::MissingRequired { key, hint }) => {
            anyhow::bail!(
                "Configuration error: Missing required setting '{}'. {}. \
                 Set the required environment variables or config.toml values directly.",
                key,
                hint
            );
        }
        Err(e) => return Err(e.into()),
    };

    // Initialize session manager before channel setup
    let session = create_session_manager(config.llm.session.clone()).await;

    // Initialize tracing for the local runtime after env/config loading.
    init_app_tracing();

    tracing::debug!("Starting IronClaw...");
    tracing::debug!("Loaded configuration for agent: {}", config.agent.name);
    tracing::debug!("LLM backend: {}", config.llm.backend);

    // ── Phase 1-5: Build all core components via AppBuilder ────────────

    let flags = AppBuilderFlags { no_db: cli.no_db };
    let components = AppBuilder::new(
        config,
        flags,
        toml_path.map(std::path::PathBuf::from),
        session.clone(),
    )
    .build_all()
    .await?;

    let config = components.config;

    // ── Orchestrator / container job manager ────────────────────────────

    let orch = ironclaw::orchestrator::setup_orchestrator(
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

    // Derive user-facing warning for the local CLI channel.
    let docker_user_warning: Option<String> = match docker_status {
        ironclaw::sandbox::DockerStatus::NotInstalled => Some(
            "Sandbox is enabled but Docker is not installed -- \
             full_job routines will fail until Docker is available."
                .to_string(),
        ),
        ironclaw::sandbox::DockerStatus::NotRunning => Some(
            "Sandbox is enabled but Docker is not running -- \
             full_job routines will fail until Docker is started."
                .to_string(),
        ),
        _ => None,
    };

    // ── Channel setup ──────────────────────────────────────────────────

    let channels = ChannelManager::new();
    let mut channel_names: Vec<String> = Vec::new();

    // Phase 0 keeps a single local REPL entrypoint while the desktop HTTP API
    // and Tauri shell are built. This avoids carrying legacy network channels
    // and gateway startup paths through the cleanup.
    let repl_channel = if let Some(ref msg) = cli.message {
        Some(ReplChannel::with_message_for_user(
            config.owner_id.clone(),
            msg.clone(),
        ))
    } else {
        let repl = ReplChannel::with_user_id(config.owner_id.clone());
        repl.suppress_banner();
        Some(repl)
    };

    if let Some(repl) = repl_channel {
        channels.add(Box::new(repl)).await;
        if cli.message.is_some() {
            tracing::debug!("Single message mode");
        } else {
            channel_names.push("repl".to_string());
            tracing::debug!("REPL mode enabled");
        }
    }

    // Register lifecycle hooks.
    let active_tool_names = components.tools.list().await;

    let hook_bootstrap = bootstrap_hooks(
        &components.hooks,
        components.workspace.as_ref(),
        &config.wasm.tools_dir,
        &config.channels.wasm_channels_dir,
        &active_tool_names,
        &[],
        &components.dev_loaded_tool_names,
    )
    .await;
    tracing::debug!(
        bundled = hook_bootstrap.bundled_hooks,
        plugin = hook_bootstrap.plugin_hooks,
        workspace = hook_bootstrap.workspace_hooks,
        outbound_webhooks = hook_bootstrap.outbound_webhooks,
        errors = hook_bootstrap.errors,
        "Lifecycle hooks initialized"
    );

    // Reuse the shared agent session manager prepared by AppBuilder.
    let session_manager = Arc::clone(&components.agent_session_manager);

    // Lazy scheduler slot — filled after Agent::new creates the Scheduler.
    // Allows CreateJobTool to dispatch local jobs via the Scheduler even though
    // the Scheduler is created after tools are registered (chicken-and-egg).
    let scheduler_slot: ironclaw::tools::builtin::SchedulerSlot =
        Arc::new(tokio::sync::RwLock::new(None));

    // Register job tools (sandbox deps auto-injected when container_job_manager is available)
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

    // ── Boot screen ────────────────────────────────────────────────────

    let boot_tool_count = components.tools.count();
    let boot_llm_model = components.llm.model_name().to_string();
    let boot_cheap_model = components
        .cheap_llm
        .as_ref()
        .map(|c| c.model_name().to_string());

    if cli.message.is_none() {
        let boot_info = ironclaw::boot_screen::BootInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            agent_name: config.agent.name.clone(),
            llm_backend: config.llm.backend.to_string(),
            llm_model: boot_llm_model,
            cheap_model: boot_cheap_model,
            db_backend: if cli.no_db {
                "none".to_string()
            } else {
                config.database.backend.to_string()
            },
            db_connected: !cli.no_db,
            tool_count: boot_tool_count,
            gateway_url: None,
            embeddings_enabled: config.embeddings.enabled,
            embeddings_provider: if config.embeddings.enabled {
                Some(config.embeddings.provider.clone())
            } else {
                None
            },
            heartbeat_enabled: config.heartbeat.enabled,
            heartbeat_interval_secs: config.heartbeat.interval_secs,
            sandbox_enabled: config.sandbox.enabled,
            docker_status,
            claude_code_enabled: config.claude_code.enabled,
            routines_enabled: config.routines.enabled,
            skills_enabled: config.skills.enabled,
            channels: channel_names,
            tunnel_url: None,
            tunnel_provider: None,
            startup_elapsed: Some(startup_start.elapsed()),
        };
        ironclaw::boot_screen::print_boot_screen(&boot_info);
    }

    // ── Run the agent ──────────────────────────────────────────────────

    let channels = Arc::new(channels);

    // Register message tool for sending messages to connected channels
    components
        .tools
        .register_message_tools(Arc::clone(&channels), components.extension_manager.clone())
        .await;

    // Snapshot memory for trace recording before the agent starts
    if let Some(ref recorder) = components.recording_handle
        && let Some(ref ws) = components.workspace
    {
        recorder.snapshot_memory(ws).await;
    }

    let http_interceptor = components
        .recording_handle
        .as_ref()
        .map(|r| r.http_interceptor());
    // Clone context_manager for the reaper before it's moved into Agent::new()
    let reaper_context_manager = Arc::clone(&components.context_manager);

    let deps = AgentDeps {
        owner_id: config.owner_id.clone(),
        store: components.db,
        llm: components.llm,
        cheap_llm: components.cheap_llm,
        safety: components.safety,
        tools: components.tools,
        workspace: components.workspace,
        extension_manager: components.extension_manager,
        skill_registry: components.skill_registry,
        skill_catalog: components.skill_catalog,
        skills_config: config.skills.clone(),
        hooks: components.hooks,
        cost_guard: components.cost_guard,
        sse_tx: None,
        http_interceptor,
        transcription: config.transcription.create_provider().map(|p| {
            Arc::new(ironclaw::llm::transcription::TranscriptionMiddleware::new(
                p,
            ))
        }),
        document_extraction: Some(Arc::new(
            ironclaw::document_extraction::DocumentExtractionMiddleware::new(),
        )),
        sandbox_readiness: if !config.sandbox.enabled {
            ironclaw::agent::routine_engine::SandboxReadiness::DisabledByConfig
        } else if docker_status.is_ok() {
            ironclaw::agent::routine_engine::SandboxReadiness::Available
        } else {
            ironclaw::agent::routine_engine::SandboxReadiness::DockerUnavailable
        },
        builder: components.builder,
        llm_backend: config.llm.backend.clone(),
        tenant_rates: Arc::new(ironclaw::tenant::TenantRateRegistry::new(
            config.agent.max_llm_concurrent_per_user.unwrap_or(4),
            config.agent.max_jobs_concurrent_per_user.unwrap_or(3),
        )),
    };

    let channels_for_warnings = Arc::clone(&channels);
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

    // Fill the scheduler slot now that Agent (and its Scheduler) exist.
    *scheduler_slot.write().await = Some(agent.scheduler());

    // Spawn sandbox reaper for orphaned container cleanup
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
                Err(e) => tracing::error!("Sandbox reaper failed to initialize: {}", e),
            }
        });
    }

    // Broadcast channel for clean shutdown of background tasks
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    agent.set_routine_engine_slot(Arc::clone(&routine_engine_slot));

    // Notify user if sandbox is unavailable (Docker missing/not running)
    if let Some(warning) = docker_user_warning {
        let channels_ref = Arc::clone(&channels_for_warnings);
        tokio::spawn(async move {
            // Delay to let channels finish connecting before sending the warning.
            // 5s is generous but avoids the message being lost on slow startups.
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            tracing::debug!("Sending sandbox-unavailable warning to connected channels");
            let response = ironclaw::channels::OutgoingResponse {
                content: format!("Warning: {warning}"),
                thread_id: None,
                attachments: Vec::new(),
                metadata: serde_json::json!({
                    "source": "system",
                    "type": "warning",
                }),
            };
            let _ = channels_ref.broadcast_all("default", response).await;
        });
    }

    agent.run().await?;

    // ── Shutdown ────────────────────────────────────────────────────────

    // Signal background tasks to gracefully shut down
    let _ = shutdown_tx.send(());

    // Shut down all stdio MCP server child processes.
    components.mcp_process_manager.shutdown_all().await;

    // Flush LLM trace recording if enabled
    if let Some(ref recorder) = components.recording_handle
        && let Err(e) = recorder.flush().await
    {
        tracing::warn!("Failed to write LLM trace: {}", e);
    }

    tracing::debug!("Agent shutdown complete");

    Ok(())
}
