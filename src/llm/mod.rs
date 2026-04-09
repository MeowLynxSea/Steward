//! LLM integration for the agent.

mod anthropic_oauth;
pub mod config;
pub mod costs;
mod disabled;
pub mod error;
pub mod failover;
pub mod oauth_helpers;
pub mod openai_codex_provider;
pub mod openai_codex_session;
mod provider;
mod reasoning;
pub mod recording;
pub mod registry;
pub mod reloadable;
pub mod response_cache;
pub mod retry;
mod rig_adapter;
pub mod session;
pub mod smart_routing;
mod token_refreshing;
pub mod transcription;

#[cfg(test)]
mod codex_test_helpers;

pub mod image_models;
pub mod reasoning_models;
pub mod vision_models;

pub use config::{
    CacheRetention, LlmConfig, OAUTH_PLACEHOLDER, OpenAiApiFormat, OpenAiCodexConfig,
    RegistryProviderConfig,
};
pub use disabled::DisabledLlmProvider;
pub use error::LlmError;
pub use openai_codex_provider::OpenAiCodexProvider;
pub use openai_codex_session::{
    OpenAiCodexDeviceCode, OpenAiCodexSession, OpenAiCodexSessionManager,
};
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentPart, FinishReason, ImageUrl,
    LlmProvider, ModelMetadata, Role, StreamDelta, ToolCall, ToolCompletionRequest,
    ToolCompletionResponse, ToolDefinition, ToolResult, generate_tool_call_id,
};
pub use reasoning::{
    ActionPlan, Reasoning, ReasoningContext, RespondOutput, RespondResult, ResponseAnomaly,
    ResponseMetadata, SILENT_REPLY_TOKEN, TOOL_INTENT_NUDGE, TRUNCATED_TOOL_CALL_NOTICE,
    TokenUsage, ToolSelection, is_silent_reply, llm_signals_tool_intent,
};
pub use recording::RecordingLlm;
pub use registry::{ProviderDefinition, ProviderProtocol, ProviderRegistry};
pub use reloadable::{
    ReloadableLlmProvider, ReloadableLlmState, ReloadableSlot, RuntimeLlmReloader,
};
pub use response_cache::{CachedProvider, ResponseCacheConfig};
pub use retry::{RetryConfig, RetryProvider};
pub use rig_adapter::RigAdapter;
pub use session::{SessionConfig, SessionManager, create_session_manager};
pub use smart_routing::{SmartRoutingConfig, SmartRoutingProvider, TaskComplexity};
pub use token_refreshing::TokenRefreshingProvider;

use std::sync::Arc;

use rig::client::CompletionClient;
use secrecy::ExposeSecret;

pub async fn create_llm_provider(
    config: &LlmConfig,
    _session: Arc<SessionManager>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    if !config.is_configured() {
        return Ok(Arc::new(DisabledLlmProvider::new()));
    }

    if config.backend == "openai_codex" {
        return Err(LlmError::RequestFailed {
            provider: "openai_codex".to_string(),
            reason:
                "OpenAI Codex uses a dedicated factory path. Use build_provider_chain() instead."
                    .to_string(),
        });
    }

    let reg_config = config
        .provider
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: config.backend.clone(),
        })?;

    create_registry_provider(reg_config)
}

fn create_registry_provider(
    config: &RegistryProviderConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    match config.protocol {
        ProviderProtocol::OpenAiCompletions => create_openai_compat_from_registry(config),
        ProviderProtocol::Anthropic => create_anthropic_from_registry(config),
        ProviderProtocol::Ollama => create_ollama_from_registry(config),
    }
}

fn create_openai_compat_from_registry(
    config: &RegistryProviderConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    use rig::providers::openai;

    let mut extra_headers = reqwest::header::HeaderMap::new();
    for (key, value) in &config.extra_headers {
        let name = match reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(header = %key, error = %e, "Skipping extra header: invalid name");
                continue;
            }
        };
        let val = match reqwest::header::HeaderValue::from_str(value) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(header = %key, error = %e, "Skipping extra header: invalid value");
                continue;
            }
        };
        extra_headers.insert(name, val);
    }

    let api_key = config
        .api_key
        .as_ref()
        .map(|k| k.expose_secret().to_string())
        .unwrap_or_else(|| "no-key".to_string());

    let mut builder = openai::Client::builder().api_key(&api_key);
    if !config.base_url.is_empty() {
        builder = builder.base_url(&config.base_url);
    }
    if !extra_headers.is_empty() {
        builder = builder.http_headers(extra_headers);
    }

    let client: openai::Client = builder.build().map_err(|e| LlmError::RequestFailed {
        provider: config.provider_id.clone(),
        reason: format!("Failed to create OpenAI-compatible client: {e}"),
    })?;

    if config.provider_id == "openai" && matches!(config.api_format, OpenAiApiFormat::Responses) {
        Ok(Arc::new(
            RigAdapter::new(client.completion_model(&config.model), &config.model)
                .with_unsupported_params(config.unsupported_params.clone()),
        ))
    } else {
        Ok(Arc::new(
            RigAdapter::new(
                client.completions_api().completion_model(&config.model),
                &config.model,
            )
            .with_unsupported_params(config.unsupported_params.clone()),
        ))
    }
}

fn create_anthropic_from_registry(
    config: &RegistryProviderConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let api_key_is_placeholder = config
        .api_key
        .as_ref()
        .is_some_and(|k| k.expose_secret() == crate::llm::config::OAUTH_PLACEHOLDER);
    if config.oauth_token.is_some() && (config.api_key.is_none() || api_key_is_placeholder) {
        return Ok(Arc::new(anthropic_oauth::AnthropicOAuthProvider::new(
            config,
        )?));
    }

    use rig::providers::anthropic;

    let api_key = config
        .api_key
        .as_ref()
        .map(|k| k.expose_secret().to_string())
        .ok_or_else(|| LlmError::AuthFailed {
            provider: config.provider_id.clone(),
        })?;

    let client: anthropic::Client = if config.base_url.is_empty() {
        anthropic::Client::new(&api_key)
    } else {
        anthropic::Client::builder()
            .api_key(&api_key)
            .base_url(&config.base_url)
            .build()
    }
    .map_err(|e| LlmError::RequestFailed {
        provider: config.provider_id.clone(),
        reason: format!("Failed to create Anthropic client: {e}"),
    })?;

    Ok(Arc::new(
        RigAdapter::new(client.completion_model(&config.model), &config.model)
            .with_cache_retention(config.cache_retention)
            .with_unsupported_params(config.unsupported_params.clone()),
    ))
}

fn create_ollama_from_registry(
    config: &RegistryProviderConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    use rig::client::Nothing;
    use rig::providers::ollama;

    let client: ollama::Client = ollama::Client::builder()
        .base_url(&config.base_url)
        .api_key(Nothing)
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: config.provider_id.clone(),
            reason: format!("Failed to create Ollama client: {e}"),
        })?;

    Ok(Arc::new(
        RigAdapter::new(client.completion_model(&config.model), &config.model)
            .with_unsupported_params(config.unsupported_params.clone()),
    ))
}

async fn create_openai_codex_provider(
    config: &LlmConfig,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let codex = config
        .openai_codex
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "openai_codex".to_string(),
        })?;

    let session_mgr = Arc::new(OpenAiCodexSessionManager::new(codex.clone())?);
    session_mgr.ensure_authenticated().await?;
    let token = session_mgr.get_access_token().await?;

    let provider = Arc::new(OpenAiCodexProvider::new(
        &codex.model,
        &codex.api_base_url,
        token.expose_secret(),
        config.request_timeout_secs,
    )?);

    Ok(Arc::new(TokenRefreshingProvider::new(
        provider,
        session_mgr,
    )))
}

async fn create_provider_for_backend_instance(
    backend: &crate::settings::BackendInstance,
    request_timeout_secs: u64,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let provider_id = backend.provider.to_ascii_lowercase();
    if matches!(
        provider_id.as_str(),
        "openai_codex" | "openai-codex" | "codex"
    ) {
        let codex = OpenAiCodexConfig {
            model: if backend.model.trim().is_empty() {
                "gpt-5.3-codex".to_string()
            } else {
                backend.model.trim().to_string()
            },
            ..OpenAiCodexConfig::default()
        };
        return create_openai_codex_provider(&LlmConfig {
            backend: "openai_codex".to_string(),
            session: SessionConfig::default(),
            provider: None,
            openai_codex: Some(codex),
            request_timeout_secs,
            cheap_backend_instance: None,
        })
        .await;
    }

    let registry = ProviderRegistry::load();
    let def = registry
        .find(&provider_id)
        .ok_or_else(|| LlmError::RequestFailed {
            provider: provider_id.clone(),
            reason: "Unknown provider".to_string(),
        })?;

    let config = RegistryProviderConfig {
        protocol: def.protocol,
        provider_id: def.id.clone(),
        api_key: backend.api_key.clone().map(secrecy::SecretString::from),
        base_url: backend
            .base_url
            .clone()
            .or_else(|| def.default_base_url.clone())
            .unwrap_or_default(),
        model: if backend.model.trim().is_empty() {
            def.default_model.clone()
        } else {
            backend.model.trim().to_string()
        },
        extra_headers: Vec::new(),
        oauth_token: if def.id == "anthropic" {
            crate::config::helpers::optional_env("ANTHROPIC_OAUTH_TOKEN")
                .ok()
                .flatten()
                .map(secrecy::SecretString::from)
        } else {
            None
        },
        cache_retention: CacheRetention::default(),
        unsupported_params: def.unsupported_params.clone(),
        api_format: if def.id == "openai" {
            OpenAiApiFormat::from_settings(backend.request_format.as_deref())
        } else {
            OpenAiApiFormat::ChatCompletions
        },
    };

    create_registry_provider(&config)
}

#[allow(clippy::type_complexity)]
pub async fn build_provider_chain(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<
    (
        Arc<dyn LlmProvider>,
        Option<Arc<dyn LlmProvider>>,
        Option<Arc<RecordingLlm>>,
    ),
    LlmError,
> {
    let primary: Arc<dyn LlmProvider> = if !config.is_configured() {
        Arc::new(DisabledLlmProvider::new())
    } else if config.backend == "openai_codex" {
        create_openai_codex_provider(config).await?
    } else {
        create_llm_provider(config, session).await?
    };

    let retry_config = RetryConfig { max_retries: 3 };
    let primary: Arc<dyn LlmProvider> = Arc::new(RetryProvider::new(primary, retry_config.clone()));

    let cheap_llm = if let Some(ref backend) = config.cheap_backend_instance {
        let cheap =
            create_provider_for_backend_instance(backend, config.request_timeout_secs).await?;
        Some(Arc::new(RetryProvider::new(cheap, retry_config.clone())) as Arc<dyn LlmProvider>)
    } else {
        None
    };

    let llm: Arc<dyn LlmProvider> = if let Some(ref cheap) = cheap_llm {
        Arc::new(SmartRoutingProvider::new(
            primary.clone(),
            cheap.clone(),
            SmartRoutingConfig::default(),
        ))
    } else {
        primary
    };

    let recording_handle = RecordingLlm::from_env(llm.clone());
    let llm: Arc<dyn LlmProvider> = if let Some(ref recorder) = recording_handle {
        Arc::clone(recorder) as Arc<dyn LlmProvider>
    } else {
        llm
    };

    Ok((llm, cheap_llm, recording_handle))
}
