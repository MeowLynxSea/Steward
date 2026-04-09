//! Models management CLI commands.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use clap::Subcommand;
use serde::Serialize;
use uuid::Uuid;

use crate::llm::ProviderRegistry;
use crate::settings::{BackendInstance, Settings};

#[derive(Subcommand, Debug, Clone)]
pub enum ModelsCommand {
    /// List supported providers
    List {
        /// Show only a specific provider
        provider: Option<String>,
        #[arg(short, long)]
        verbose: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show current model configuration
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Set the current major backend model
    Set { model: String },
    /// Set the current major backend provider
    SetProvider {
        provider: String,
        #[arg(long)]
        model: Option<String>,
    },
}

#[derive(Debug, Serialize)]
struct ProviderRow {
    id: String,
    description: String,
    default_model: String,
    protocol: String,
    base_url_env: Option<String>,
    api_key_env: Option<String>,
}

#[derive(Debug, Serialize)]
struct StatusRow {
    configured: bool,
    provider: Option<String>,
    model: Option<String>,
    request_format: Option<String>,
    cheap_provider: Option<String>,
    cheap_model: Option<String>,
}

pub async fn run_models_command(
    cmd: ModelsCommand,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    match cmd {
        ModelsCommand::List {
            provider,
            verbose,
            json,
        } => {
            if let Some(provider) = provider {
                cmd_show_provider(&provider, verbose, json)
            } else {
                cmd_list_providers(verbose, json)
            }
        }
        ModelsCommand::Status { json } => cmd_status(json, config_path),
        ModelsCommand::Set { model } => cmd_set_model(&model, config_path),
        ModelsCommand::SetProvider { provider, model } => {
            cmd_set_provider(&provider, model.as_deref(), config_path)
        }
    }
}

fn normalize_provider_id(provider: &str) -> Option<String> {
    let normalized = provider.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "openai" => Some("openai".to_string()),
        "openai_codex" | "openai-codex" | "codex" => Some("openai_codex".to_string()),
        "anthropic" => Some("anthropic".to_string()),
        "groq" => Some("groq".to_string()),
        "openrouter" => Some("openrouter".to_string()),
        "ollama" => Some("ollama".to_string()),
        _ => ProviderRegistry::load()
            .find(&normalized)
            .map(|def| def.id.clone()),
    }
}

fn config_toml_path() -> PathBuf {
    crate::bootstrap::steward_base_dir().join("config.toml")
}

fn load_settings(config_path: Option<&Path>) -> Settings {
    let path = config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(config_toml_path);
    Settings::load_toml(&path)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn save_settings(settings: &Settings, config_path: Option<&Path>) -> anyhow::Result<()> {
    let path = config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(config_toml_path);
    settings
        .save_toml(&path)
        .map_err(|error| anyhow!("failed to save {}: {}", path.display(), error))
}

fn list_provider_rows(verbose: bool) -> Vec<ProviderRow> {
    let registry = ProviderRegistry::load();
    let mut rows: Vec<ProviderRow> = registry
        .all()
        .iter()
        .map(|def| ProviderRow {
            id: def.id.clone(),
            description: def.description.clone(),
            default_model: def.default_model.clone(),
            protocol: format!("{:?}", def.protocol),
            base_url_env: verbose.then(|| def.base_url_env.clone()).flatten(),
            api_key_env: verbose.then(|| def.api_key_env.clone()).flatten(),
        })
        .collect();

    rows.push(ProviderRow {
        id: "openai_codex".to_string(),
        description: "OpenAI Codex via dedicated auth flow".to_string(),
        default_model: "gpt-5.3-codex".to_string(),
        protocol: "Dedicated".to_string(),
        base_url_env: verbose
            .then(|| Some("OPENAI_CODEX_API_URL".to_string()))
            .flatten(),
        api_key_env: None,
    });

    rows
}

fn cmd_list_providers(verbose: bool, json: bool) -> anyhow::Result<()> {
    let rows = list_provider_rows(verbose);
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    for row in rows {
        println!("{}  {}", row.id, row.description);
        println!("  default model: {}", row.default_model);
        if verbose {
            println!("  protocol: {}", row.protocol);
            if let Some(base_url_env) = row.base_url_env {
                println!("  base url env: {}", base_url_env);
            }
            if let Some(api_key_env) = row.api_key_env {
                println!("  api key env: {}", api_key_env);
            }
        }
    }
    Ok(())
}

fn cmd_show_provider(provider: &str, verbose: bool, json: bool) -> anyhow::Result<()> {
    let normalized = normalize_provider_id(provider).ok_or_else(|| {
        anyhow!(
            "unknown provider '{}'; expected openai, openai_codex, anthropic, groq, openrouter, or ollama",
            provider
        )
    })?;

    let row = list_provider_rows(verbose)
        .into_iter()
        .find(|row| row.id == normalized)
        .context("provider not found")?;

    if json {
        println!("{}", serde_json::to_string_pretty(&row)?);
    } else {
        println!("Provider: {}", row.id);
        println!("Description: {}", row.description);
        println!("Default model: {}", row.default_model);
        println!("Protocol: {}", row.protocol);
        if let Some(base_url_env) = row.base_url_env {
            println!("Base URL env: {}", base_url_env);
        }
        if let Some(api_key_env) = row.api_key_env {
            println!("API key env: {}", api_key_env);
        }
    }

    Ok(())
}

fn current_status(settings: &Settings) -> StatusRow {
    let major = settings.major_backend();
    let cheap = if settings.cheap_model_uses_primary {
        None
    } else {
        settings.cheap_backend()
    };

    StatusRow {
        configured: major.is_some(),
        provider: major.map(|backend| backend.provider.clone()),
        model: major.map(|backend| backend.model.clone()),
        request_format: major.and_then(|backend| backend.request_format.clone()),
        cheap_provider: cheap.map(|backend| backend.provider.clone()),
        cheap_model: cheap.map(|backend| backend.model.clone()),
    }
}

fn cmd_status(json: bool, config_path: Option<&Path>) -> anyhow::Result<()> {
    let settings = load_settings(config_path);
    let status = current_status(&settings);

    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    if let Some(provider) = status.provider {
        println!("Primary provider: {}", provider);
        println!(
            "Primary model: {}",
            status
                .model
                .unwrap_or_else(|| "(provider default)".to_string())
        );
        if let Some(request_format) = status.request_format {
            println!("Request format: {}", request_format);
        }
        if let Some(cheap_provider) = status.cheap_provider {
            println!("Cheap provider: {}", cheap_provider);
            println!(
                "Cheap model: {}",
                status
                    .cheap_model
                    .unwrap_or_else(|| "(provider default)".to_string())
            );
        } else if settings.cheap_model_uses_primary {
            println!("Cheap provider: reuse primary");
        }
    } else {
        println!("No backend configured. Complete onboarding first.");
    }

    Ok(())
}

fn cmd_set_model(model: &str, config_path: Option<&Path>) -> anyhow::Result<()> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("model cannot be empty"));
    }

    let mut settings = load_settings(config_path);
    let major_id = settings
        .major_backend_id
        .clone()
        .or_else(|| settings.backends.first().map(|backend| backend.id.clone()))
        .ok_or_else(|| anyhow!("no backend configured; set a provider first"))?;

    let (provider, backend_id) = {
        let backend = settings
            .backends
            .iter_mut()
            .find(|backend| backend.id == major_id)
            .context("major backend not found")?;
        backend.model = trimmed.to_string();
        (backend.provider.clone(), backend.id.clone())
    };
    settings.major_backend_id = Some(backend_id);
    settings.onboard_completed = true;
    save_settings(&settings, config_path)?;

    println!("Model set to '{}' for provider '{}'", trimmed, provider);
    Ok(())
}

fn ensure_backend(
    settings: &mut Settings,
    provider: &str,
    model: Option<&str>,
) -> anyhow::Result<String> {
    if let Some(existing) = settings
        .backends
        .iter_mut()
        .find(|backend| backend.provider.eq_ignore_ascii_case(provider))
    {
        if let Some(model) = model {
            existing.model = model.to_string();
        } else if existing.model.trim().is_empty() {
            existing.model = if provider == "openai_codex" {
                "gpt-5.3-codex".to_string()
            } else {
                ProviderRegistry::load()
                    .find(provider)
                    .map(|def| def.default_model.clone())
                    .unwrap_or_default()
            };
        }
        return Ok(existing.id.clone());
    }

    let default_model = if let Some(model) = model {
        model.to_string()
    } else if provider == "openai_codex" {
        "gpt-5.3-codex".to_string()
    } else {
        ProviderRegistry::load()
            .find(provider)
            .map(|def| def.default_model.clone())
            .ok_or_else(|| anyhow!("unknown provider '{}'", provider))?
    };

    let backend = BackendInstance {
        id: Uuid::new_v4().to_string(),
        provider: provider.to_string(),
        api_key: None,
        base_url: None,
        model: default_model,
        request_format: (provider == "openai").then(|| "chat_completions".to_string()),
    };
    let id = backend.id.clone();
    settings.backends.push(backend);
    Ok(id)
}

fn cmd_set_provider(
    provider: &str,
    model: Option<&str>,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    let provider = normalize_provider_id(provider).ok_or_else(|| {
        anyhow!(
            "unknown provider '{}'; expected openai, openai_codex, anthropic, groq, openrouter, or ollama",
            provider
        )
    })?;

    let mut settings = load_settings(config_path);
    let backend_id = ensure_backend(&mut settings, &provider, model)?;
    settings.major_backend_id = Some(backend_id);
    settings.onboard_completed = true;
    save_settings(&settings, config_path)?;

    println!("Primary provider set to '{}'", provider);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reports_unconfigured_without_backends() {
        let status = current_status(&Settings::default());
        assert!(!status.configured);
        assert!(status.provider.is_none());
    }
}
