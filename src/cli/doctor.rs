//! `steward doctor` diagnostics.

use std::path::PathBuf;

use crate::bootstrap::steward_base_dir;
use crate::cli::fmt;
use crate::settings::Settings;

pub async fn run_doctor_command() -> anyhow::Result<()> {
    println!();
    println!("  {}Steward Doctor{}", fmt::bold(), fmt::reset());

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let settings = Settings::load();

    section_header("Core");
    check(
        "Settings file",
        check_settings_file(),
        &mut passed,
        &mut failed,
        &mut skipped,
    );
    check(
        "LLM configuration",
        check_llm_config(&settings),
        &mut passed,
        &mut failed,
        &mut skipped,
    );
    check(
        "Database backend",
        check_database().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );
    check(
        "Workspace directory",
        check_workspace_dir(),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    section_header("Features");
    check(
        "Embeddings",
        check_embeddings(&settings),
        &mut passed,
        &mut failed,
        &mut skipped,
    );
    check(
        "Routines config",
        check_routines_config(),
        &mut passed,
        &mut failed,
        &mut skipped,
    );
    check(
        "Desktop transport",
        check_desktop_transport(&settings),
        &mut passed,
        &mut failed,
        &mut skipped,
    );
    check(
        "MCP servers",
        check_mcp_config().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );
    check(
        "Skills",
        check_skills().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );
    check(
        "Secrets",
        check_secrets(&settings),
        &mut passed,
        &mut failed,
        &mut skipped,
    );
    check(
        "Service",
        check_service_installed(),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    section_header("External");
    check(
        "cloudflared",
        check_binary("cloudflared", &["--version"]),
        &mut passed,
        &mut failed,
        &mut skipped,
    );
    check(
        "ngrok",
        check_binary("ngrok", &["version"]),
        &mut passed,
        &mut failed,
        &mut skipped,
    );
    check(
        "tailscale",
        check_binary("tailscale", &["version"]),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    println!();
    println!(
        "  {}{} passed{}, {}{} failed{}, {}{} skipped{}",
        fmt::success(),
        passed,
        fmt::reset(),
        if failed > 0 { fmt::error() } else { fmt::dim() },
        failed,
        fmt::reset(),
        fmt::dim(),
        skipped,
        fmt::reset(),
    );

    if failed > 0 {
        println!("\n  Some checks failed. This is normal if you do not use those features.");
    }

    Ok(())
}

fn section_header(name: &str) {
    println!();
    println!("  {}", fmt::separator(36));
    println!("  {}{}{}", fmt::bold(), name, fmt::reset());
    println!();
}

fn check(name: &str, result: CheckResult, passed: &mut u32, failed: &mut u32, skipped: &mut u32) {
    match result {
        CheckResult::Pass(detail) => {
            *passed += 1;
            println!(
                "{}",
                fmt::check_line(fmt::StatusKind::Pass, name, &detail, 18)
            );
        }
        CheckResult::Fail(detail) => {
            *failed += 1;
            println!(
                "{}",
                fmt::check_line(fmt::StatusKind::Fail, name, &detail, 18)
            );
        }
        CheckResult::Skip(reason) => {
            *skipped += 1;
            println!(
                "{}",
                fmt::check_line(fmt::StatusKind::Skip, name, &reason, 18)
            );
        }
    }
}

enum CheckResult {
    Pass(String),
    Fail(String),
    Skip(String),
}

fn check_settings_file() -> CheckResult {
    let path = Settings::default_path();
    if !path.exists() {
        return CheckResult::Pass("no settings file (defaults will be used)".into());
    }

    match std::fs::read_to_string(&path) {
        Ok(data) => match serde_json::from_str::<serde_json::Value>(&data) {
            Ok(_) => CheckResult::Pass(format!("valid ({})", path.display())),
            Err(error) => CheckResult::Fail(format!(
                "settings.json is malformed: {}. Fix or delete {}",
                error,
                path.display()
            )),
        },
        Err(error) => CheckResult::Fail(format!("cannot read {}: {}", path.display(), error)),
    }
}

fn check_llm_config(settings: &Settings) -> CheckResult {
    match crate::llm::LlmConfig::resolve(settings) {
        Ok(config) if !config.is_configured() => {
            CheckResult::Skip("no backend configured; onboarding will be shown".into())
        }
        Ok(config) => {
            let model = config
                .provider
                .as_ref()
                .map(|provider| provider.model.as_str())
                .or_else(|| {
                    config
                        .openai_codex
                        .as_ref()
                        .map(|codex| codex.model.as_str())
                })
                .unwrap_or("unknown");
            CheckResult::Pass(format!("backend={}, model={}", config.backend, model))
        }
        Err(error) => CheckResult::Fail(format!("LLM config error: {error}")),
    }
}

async fn check_database() -> CheckResult {
    let backend = std::env::var("DATABASE_BACKEND")
        .ok()
        .unwrap_or_else(|| "libsql".into());

    match backend.as_str() {
        "libsql" | "turso" | "sqlite" => {
            let path = std::env::var("LIBSQL_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| crate::config::default_libsql_path());

            if path.exists() {
                CheckResult::Pass(format!("libSQL database exists ({})", path.display()))
            } else {
                CheckResult::Pass(format!(
                    "libSQL database not found at {} (will be created on first run)",
                    path.display()
                ))
            }
        }
        _ => CheckResult::Fail("unsupported database backend configured".into()),
    }
}

fn check_workspace_dir() -> CheckResult {
    let dir = steward_base_dir();

    if dir.exists() {
        if dir.is_dir() {
            CheckResult::Pass(dir.display().to_string())
        } else {
            CheckResult::Fail(format!("{} exists but is not a directory", dir.display()))
        }
    } else {
        CheckResult::Pass(format!("{} will be created on first run", dir.display()))
    }
}

fn check_embeddings(settings: &Settings) -> CheckResult {
    match crate::config::EmbeddingsConfig::resolve(settings) {
        Ok(config) => {
            if !config.enabled {
                return CheckResult::Skip("disabled (set EMBEDDING_ENABLED=true)".into());
            }

            let credentials_ready = match config.provider.as_str() {
                "ollama" => true,
                "openai" => config.openai_api_key().is_some(),
                provider => {
                    return CheckResult::Fail(format!(
                        "unsupported embeddings provider '{}'; expected openai or ollama",
                        provider
                    ));
                }
            };

            if credentials_ready {
                CheckResult::Pass(format!(
                    "provider={}, model={}",
                    config.provider, config.model
                ))
            } else {
                CheckResult::Fail("provider=openai but OPENAI_API_KEY is missing".into())
            }
        }
        Err(error) => CheckResult::Fail(format!("config error: {error}")),
    }
}

fn check_routines_config() -> CheckResult {
    match crate::config::RoutineConfig::resolve() {
        Ok(config) if config.enabled => CheckResult::Pass(format!(
            "enabled (interval={}s, max_concurrent={})",
            config.cron_check_interval_secs, config.max_concurrent_routines
        )),
        Ok(_) => CheckResult::Skip("disabled".into()),
        Err(error) => CheckResult::Fail(format!("config error: {error}")),
    }
}

fn check_desktop_transport(settings: &Settings) -> CheckResult {
    let owner_id = match crate::config::resolve_owner_id(settings) {
        Ok(owner_id) => owner_id,
        Err(error) => return CheckResult::Fail(format!("config error: {error}")),
    };
    match crate::config::ChannelsConfig::resolve(settings, &owner_id) {
        Ok(channels) if channels.desktop.tauri_ipc => CheckResult::Pass("tauri-ipc enabled".into()),
        Ok(_) => CheckResult::Fail("tauri-ipc disabled".into()),
        Err(error) => CheckResult::Fail(format!("config error: {error}")),
    }
}

async fn check_mcp_config() -> CheckResult {
    match crate::tools::mcp::config::load_mcp_servers().await {
        Ok(servers) => CheckResult::Pass(format!(
            "{} enabled / {} configured",
            servers
                .servers
                .iter()
                .filter(|server| server.enabled)
                .count(),
            servers.servers.len()
        )),
        Err(error) => CheckResult::Skip(format!("not configured ({error})")),
    }
}

async fn check_skills() -> CheckResult {
    let dir = crate::bootstrap::steward_base_dir().join("skills");
    if dir.exists() {
        CheckResult::Pass(dir.display().to_string())
    } else {
        CheckResult::Skip(format!("directory not found ({})", dir.display()))
    }
}

fn check_secrets(_settings: &Settings) -> CheckResult {
    if std::env::var("SECRETS_MASTER_KEY").is_ok() {
        CheckResult::Pass("configured via environment".into())
    } else {
        CheckResult::Skip("environment key not set (keychain may still be configured)".into())
    }
}

fn check_service_installed() -> CheckResult {
    CheckResult::Skip("service diagnostics not implemented for desktop runtime".into())
}

fn check_binary(name: &str, args: &[&str]) -> CheckResult {
    match std::process::Command::new(name).args(args).output() {
        Ok(output) if output.status.success() => CheckResult::Pass("installed".into()),
        Ok(_) => CheckResult::Fail("installed but returned non-zero status".into()),
        Err(_) => CheckResult::Skip("not installed".into()),
    }
}
