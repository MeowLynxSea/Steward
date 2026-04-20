use std::path::PathBuf;

use secrecy::SecretString;

use crate::bootstrap::steward_base_dir;
use crate::config::helpers::{optional_env, parse_optional_env, validate_base_url};
use crate::error::ConfigError;
use crate::llm::config::*;
use crate::llm::registry::ProviderRegistry;
use crate::llm::session::SessionConfig;
use crate::settings::{BackendInstance, Settings};

const UNCONFIGURED_BACKEND: &str = "unconfigured";

impl LlmConfig {
    #[cfg(feature = "libsql")]
    pub fn for_testing() -> Self {
        Self {
            backend: UNCONFIGURED_BACKEND.to_string(),
            session: SessionConfig {
                auth_base_url: "http://localhost:0".to_string(),
                session_path: std::env::temp_dir().join("steward-test-session.json"),
            },
            provider: None,
            openai_codex: None,
            request_timeout_secs: 120,
            cheap_backend_instance: None,
        }
    }

    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let request_timeout_secs = parse_optional_env("LLM_REQUEST_TIMEOUT_SECS", 120)?;
        let session = SessionConfig {
            auth_base_url: "https://auth.openai.com".to_string(),
            session_path: default_session_path(),
        };

        let Some(major) = settings.major_backend() else {
            return Ok(Self {
                backend: UNCONFIGURED_BACKEND.to_string(),
                session,
                provider: None,
                openai_codex: None,
                request_timeout_secs,
                cheap_backend_instance: None,
            });
        };

        let backend = normalize_provider_id(&major.provider)?;
        let cheap_backend_instance = if settings.cheap_model_uses_primary {
            None
        } else {
            settings
                .cheap_backend()
                .filter(|cheap| cheap.id != major.id)
                .cloned()
        };

        if backend == "openai_codex" {
            return Ok(Self {
                backend,
                session,
                provider: None,
                openai_codex: Some(resolve_openai_codex(major)?),
                request_timeout_secs,
                cheap_backend_instance,
            });
        }

        Ok(Self {
            backend,
            session,
            provider: Some(resolve_registry_provider(major)?),
            openai_codex: None,
            request_timeout_secs,
            cheap_backend_instance,
        })
    }
}

fn normalize_provider_id(provider: &str) -> Result<String, ConfigError> {
    match provider.to_ascii_lowercase().as_str() {
        "openai" => Ok("openai".to_string()),
        "openai_codex" | "openai-codex" | "codex" => Ok("openai_codex".to_string()),
        "anthropic" => Ok("anthropic".to_string()),
        "groq" => Ok("groq".to_string()),
        "openrouter" => Ok("openrouter".to_string()),
        "ollama" => Ok("ollama".to_string()),
        other => Err(ConfigError::InvalidValue {
            key: "backends[].provider".to_string(),
            message: format!(
                "unsupported provider '{}'; expected openai, openai_codex, anthropic, groq, openrouter, or ollama",
                other
            ),
        }),
    }
}

fn resolve_registry_provider(
    backend: &BackendInstance,
) -> Result<RegistryProviderConfig, ConfigError> {
    let canonical_id = normalize_provider_id(&backend.provider)?;
    if canonical_id == "openai_codex" {
        return Err(ConfigError::InvalidValue {
            key: "backends[].provider".to_string(),
            message: "openai_codex must use the dedicated codex path".to_string(),
        });
    }

    let registry = ProviderRegistry::load();
    let def = registry
        .find(&canonical_id)
        .ok_or_else(|| ConfigError::InvalidValue {
            key: "backends[].provider".to_string(),
            message: format!("unknown provider '{}'", canonical_id),
        })?;

    let base_url = backend
        .base_url
        .clone()
        .or_else(|| {
            def.base_url_env
                .as_deref()
                .and_then(|env| optional_env(env).ok().flatten())
        })
        .or_else(|| def.default_base_url.clone())
        .unwrap_or_default();

    if !base_url.is_empty() {
        let field = def.base_url_env.as_deref().unwrap_or("LLM_BASE_URL");
        validate_base_url(&base_url, field)?;
    }

    let api_key = backend
        .api_key
        .clone()
        .filter(|value| !value.trim().is_empty())
        .map(SecretString::from)
        .or_else(|| {
            def.api_key_env
                .as_deref()
                .and_then(|env| optional_env(env).ok().flatten())
                .map(SecretString::from)
        });

    let oauth_token = if canonical_id == "anthropic" {
        optional_env("ANTHROPIC_OAUTH_TOKEN")?.map(SecretString::from)
    } else {
        None
    };

    let api_key = if api_key.is_none() && oauth_token.is_some() {
        Some(SecretString::from(OAUTH_PLACEHOLDER.to_string()))
    } else {
        api_key
    };

    let model = if backend.model.trim().is_empty() {
        def.default_model.clone()
    } else {
        backend.model.trim().to_string()
    };

    let api_format = if canonical_id == "openai" {
        OpenAiApiFormat::from_settings(backend.request_format.as_deref())
    } else {
        OpenAiApiFormat::ChatCompletions
    };

    Ok(RegistryProviderConfig {
        protocol: def.protocol,
        provider_id: canonical_id,
        api_key,
        base_url,
        model,
        extra_headers: Vec::new(),
        oauth_token,
        cache_retention: CacheRetention::default(),
        unsupported_params: def.unsupported_params.clone(),
        api_format,
        context_length: backend.context_length,
    })
}

fn resolve_openai_codex(backend: &BackendInstance) -> Result<OpenAiCodexConfig, ConfigError> {
    let model = if backend.model.trim().is_empty() {
        "gpt-5.3-codex".to_string()
    } else {
        backend.model.trim().to_string()
    };

    let auth_endpoint = optional_env("OPENAI_CODEX_AUTH_URL")?
        .unwrap_or_else(|| "https://auth.openai.com".to_string());
    validate_base_url(&auth_endpoint, "OPENAI_CODEX_AUTH_URL")?;

    let api_base_url = optional_env("OPENAI_CODEX_API_URL")?
        .unwrap_or_else(|| "https://chatgpt.com/backend-api/codex".to_string());
    validate_base_url(&api_base_url, "OPENAI_CODEX_API_URL")?;

    let client_id = optional_env("OPENAI_CODEX_CLIENT_ID")?
        .unwrap_or_else(|| "app_EMoamEEZ73f0CkXaXp7hrann".to_string());

    Ok(OpenAiCodexConfig {
        model,
        auth_endpoint,
        api_base_url,
        client_id,
        session_path: steward_base_dir().join("openai_codex_session.json"),
        token_refresh_margin_secs: parse_optional_env(
            "OPENAI_CODEX_TOKEN_REFRESH_MARGIN_SECS",
            300,
        )?,
        context_length: backend.context_length,
    })
}

pub fn default_session_path() -> PathBuf {
    steward_base_dir().join("session.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend(
        id: &str,
        provider: &str,
        model: &str,
        request_format: Option<&str>,
    ) -> BackendInstance {
        BackendInstance {
            id: id.to_string(),
            provider: provider.to_string(),
            api_key: None,
            base_url: None,
            model: model.to_string(),
            request_format: request_format.map(str::to_string),
            context_length: None,
        }
    }

    #[test]
    fn resolves_unconfigured_when_no_backends_exist() {
        let config = LlmConfig::resolve(&Settings::default()).expect("resolve");
        assert_eq!(config.backend, "unconfigured");
        assert!(config.provider.is_none());
        assert!(config.openai_codex.is_none());
    }

    #[test]
    fn resolves_openai_chat_and_responses() {
        let chat = Settings {
            backends: vec![backend(
                "b1",
                "openai",
                "gpt-5-mini",
                Some("chat_completions"),
            )],
            major_backend_id: Some("b1".to_string()),
            ..Default::default()
        };
        let responses = Settings {
            backends: vec![backend("b1", "openai", "gpt-5-mini", Some("responses"))],
            major_backend_id: Some("b1".to_string()),
            ..Default::default()
        };

        let chat_cfg = LlmConfig::resolve(&chat).expect("resolve");
        let resp_cfg = LlmConfig::resolve(&responses).expect("resolve");

        assert!(matches!(
            chat_cfg.provider.as_ref().map(|cfg| cfg.api_format),
            Some(OpenAiApiFormat::ChatCompletions)
        ));
        assert!(matches!(
            resp_cfg.provider.as_ref().map(|cfg| cfg.api_format),
            Some(OpenAiApiFormat::Responses)
        ));
    }

    #[test]
    fn resolves_codex_as_major_backend() {
        let settings = Settings {
            backends: vec![backend("b1", "openai_codex", "gpt-5.3-codex", None)],
            major_backend_id: Some("b1".to_string()),
            ..Default::default()
        };

        let config = LlmConfig::resolve(&settings).expect("resolve");
        assert_eq!(config.backend, "openai_codex");
        assert!(config.openai_codex.is_some());
    }
}
