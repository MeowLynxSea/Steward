//! LLM configuration types.

use std::path::PathBuf;

use secrecy::SecretString;

use crate::bootstrap::steward_base_dir;
use crate::llm::registry::ProviderProtocol;
use crate::llm::session::SessionConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiApiFormat {
    ChatCompletions,
    Responses,
}

impl OpenAiApiFormat {
    pub fn from_settings(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("responses") => Self::Responses,
            _ => Self::ChatCompletions,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
        }
    }
}

/// Sentinel value used as `api_key` when only an OAuth token is present.
pub const OAUTH_PLACEHOLDER: &str = "oauth-placeholder";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheRetention {
    None,
    #[default]
    Short,
    Long,
}

impl std::str::FromStr for CacheRetention {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" | "off" | "disabled" => Ok(Self::None),
            "short" | "5m" | "ephemeral" => Ok(Self::Short),
            "long" | "1h" => Ok(Self::Long),
            _ => Err(format!(
                "invalid cache retention '{}', expected one of: none, short, long",
                s
            )),
        }
    }
}

impl std::fmt::Display for CacheRetention {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Short => write!(f, "short"),
            Self::Long => write!(f, "long"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegistryProviderConfig {
    pub protocol: ProviderProtocol,
    pub provider_id: String,
    pub api_key: Option<SecretString>,
    pub base_url: String,
    pub model: String,
    pub extra_headers: Vec<(String, String)>,
    pub oauth_token: Option<SecretString>,
    pub cache_retention: CacheRetention,
    pub unsupported_params: Vec<String>,
    pub api_format: OpenAiApiFormat,
}

#[derive(Debug, Clone)]
pub struct OpenAiCodexConfig {
    pub model: String,
    pub auth_endpoint: String,
    pub api_base_url: String,
    pub client_id: String,
    pub session_path: PathBuf,
    pub token_refresh_margin_secs: u64,
}

impl Default for OpenAiCodexConfig {
    fn default() -> Self {
        Self {
            model: "gpt-5.3-codex".to_string(),
            auth_endpoint: "https://auth.openai.com".to_string(),
            api_base_url: "https://chatgpt.com/backend-api/codex".to_string(),
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann".to_string(),
            session_path: steward_base_dir().join("openai_codex_session.json"),
            token_refresh_margin_secs: 300,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// `"unconfigured"` when the user has not set up any backend yet.
    pub backend: String,
    pub session: SessionConfig,
    pub provider: Option<RegistryProviderConfig>,
    pub openai_codex: Option<OpenAiCodexConfig>,
    pub request_timeout_secs: u64,
    pub cheap_backend_instance: Option<crate::settings::BackendInstance>,
}

impl LlmConfig {
    pub fn is_configured(&self) -> bool {
        self.backend != "unconfigured"
    }
}
