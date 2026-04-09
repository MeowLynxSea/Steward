//! Minimal session manager retained for API compatibility.
//!
//! Legacy NEAR AI session auth has been removed. The remaining providers use
//! direct API keys or the dedicated OpenAI Codex session manager.

use std::path::PathBuf;
use std::sync::Arc;

use secrecy::SecretString;

use crate::llm::error::LlmError;

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub auth_base_url: String,
    pub session_path: PathBuf,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            auth_base_url: "https://auth.openai.com".to_string(),
            session_path: PathBuf::from("session.json"),
        }
    }
}

#[derive(Debug)]
pub struct SessionManager {
    config: SessionConfig,
}

impl SessionManager {
    pub fn new(config: SessionConfig) -> Self {
        Self { config }
    }

    pub async fn new_async(config: SessionConfig) -> Self {
        Self::new(config)
    }

    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    pub async fn attach_store(&self, _store: Arc<dyn crate::db::Database>, _user_id: &str) {}

    pub async fn get_token(&self) -> Result<SecretString, LlmError> {
        Err(LlmError::AuthFailed {
            provider: "session".to_string(),
        })
    }

    pub async fn has_token(&self) -> bool {
        false
    }

    pub async fn ensure_authenticated(&self) -> Result<(), LlmError> {
        Err(LlmError::AuthFailed {
            provider: "session".to_string(),
        })
    }

    pub async fn handle_auth_failure(&self) -> Result<(), LlmError> {
        Err(LlmError::SessionRenewalFailed {
            provider: "session".to_string(),
            reason: "Interactive session authentication is no longer supported".to_string(),
        })
    }
}

pub async fn create_session_manager(config: SessionConfig) -> Arc<SessionManager> {
    Arc::new(SessionManager::new_async(config).await)
}
