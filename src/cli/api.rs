//! CLI subcommand definitions for `ironcowork api`.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use anyhow::Context;
use clap::{Args, Subcommand};

use crate::api::{ApiState, DEFAULT_API_HOST, DEFAULT_API_PORT, run_api};
use crate::config::Config;
use crate::db::connect_from_config;
use crate::runtime_events::SseManager;
use crate::task_runtime::TaskRuntime;

#[derive(Subcommand, Debug, Clone)]
pub enum ApiCommand {
    /// Run the local HTTP API for the desktop shell and browser clients.
    Serve(ApiServeArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ApiServeArgs {
    /// Bind host. Phase 1 only supports loopback addresses.
    #[arg(long, default_value_t = DEFAULT_API_HOST)]
    pub host: IpAddr,

    /// Bind port for the local API server.
    #[arg(long, default_value_t = DEFAULT_API_PORT)]
    pub port: u16,
}

pub async fn run_api_command(
    command: &ApiCommand,
    toml_path: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    match command {
        ApiCommand::Serve(args) => run_api_serve(args, toml_path).await,
    }
}

async fn run_api_serve(
    args: &ApiServeArgs,
    toml_path: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let bind_addr = SocketAddr::new(args.host, args.port);
    let config = Config::from_env_with_toml(toml_path).await?;
    let database = connect_from_config(&config.database).await?;

    let state = ApiState::new(
        config.owner_id.clone(),
        bind_addr,
        database.clone(),
        Arc::new(SseManager::new()),
        Some(Arc::new(TaskRuntime::with_store(config.owner_id, database))),
        None,
        None,
        None,
    );

    run_api(bind_addr, state)
        .await
        .with_context(|| format!("failed to run local api on {}", bind_addr))
}
