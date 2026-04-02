use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::config::hydrate_llm_keys_from_secrets;
use crate::db::SettingsStore;
use crate::llm::{
    CompletionRequest, CompletionResponse, LlmConfig, LlmError, LlmProvider, ModelMetadata,
    RecordingLlm, SessionManager, ToolCompletionRequest, ToolCompletionResponse,
    build_provider_chain,
};
use crate::secrets::SecretsStore;
use crate::settings::Settings;

#[derive(Clone, Copy)]
pub enum ReloadableSlot {
    Primary,
    Cheap,
}

pub struct ReloadableLlmState {
    primary: RwLock<Arc<dyn LlmProvider>>,
    cheap: RwLock<Arc<dyn LlmProvider>>,
}

impl ReloadableLlmState {
    pub fn new(primary: Arc<dyn LlmProvider>, cheap: Arc<dyn LlmProvider>) -> Self {
        Self {
            primary: RwLock::new(primary),
            cheap: RwLock::new(cheap),
        }
    }

    pub fn provider(&self, slot: ReloadableSlot) -> Arc<dyn LlmProvider> {
        match slot {
            ReloadableSlot::Primary => self.primary.read().expect("primary llm lock").clone(),
            ReloadableSlot::Cheap => self.cheap.read().expect("cheap llm lock").clone(),
        }
    }

    pub fn replace(&self, primary: Arc<dyn LlmProvider>, cheap: Arc<dyn LlmProvider>) {
        *self.primary.write().expect("primary llm lock") = primary;
        *self.cheap.write().expect("cheap llm lock") = cheap;
    }
}

pub struct ReloadableLlmProvider {
    state: Arc<ReloadableLlmState>,
    slot: ReloadableSlot,
}

impl ReloadableLlmProvider {
    pub fn new(state: Arc<ReloadableLlmState>, slot: ReloadableSlot) -> Self {
        Self { state, slot }
    }

    fn current(&self) -> Arc<dyn LlmProvider> {
        self.state.provider(self.slot)
    }
}

#[async_trait]
impl LlmProvider for ReloadableLlmProvider {
    fn model_name(&self) -> &str {
        "reloadable"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.current().cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.current().complete(request).await
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.current().complete_with_tools(request).await
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.current().list_models().await
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        self.current().model_metadata().await
    }

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        self.current().effective_model_name(requested_model)
    }

    fn active_model_name(&self) -> String {
        self.current().active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.current().set_model(model)
    }

    fn cache_write_multiplier(&self) -> Decimal {
        self.current().cache_write_multiplier()
    }

    fn cache_read_discount(&self) -> Decimal {
        self.current().cache_read_discount()
    }
}

pub struct RuntimeLlmReloader {
    state: Arc<ReloadableLlmState>,
    session: Arc<SessionManager>,
    owner_id: String,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
}

impl RuntimeLlmReloader {
    pub fn new(
        state: Arc<ReloadableLlmState>,
        session: Arc<SessionManager>,
        owner_id: String,
        secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    ) -> Self {
        Self {
            state,
            session,
            owner_id,
            secrets_store,
        }
    }

    pub async fn reload_from_settings(
        &self,
        settings: &Settings,
    ) -> Result<Option<Arc<RecordingLlm>>, LlmError> {
        let mut hydrated = settings.clone();
        if let Some(secrets) = self.secrets_store.as_ref() {
            hydrate_llm_keys_from_secrets(&mut hydrated, secrets.as_ref(), &self.owner_id).await;
        }

        let llm_config = LlmConfig::resolve(&hydrated).map_err(|error| LlmError::RequestFailed {
            provider: "settings".to_string(),
            reason: error.to_string(),
        })?;
        let (primary, cheap, recording_handle) =
            build_provider_chain(&llm_config, self.session.clone()).await?;
        let cheap = cheap.unwrap_or_else(|| primary.clone());
        self.state.replace(primary, cheap);
        Ok(recording_handle)
    }

    pub async fn reload_from_store(
        &self,
        store: &(dyn SettingsStore + Sync),
    ) -> Result<Option<Arc<RecordingLlm>>, LlmError> {
        let map = store
            .get_all_settings(&self.owner_id)
            .await
            .map_err(|error| LlmError::RequestFailed {
                provider: "settings".to_string(),
                reason: error.to_string(),
            })?;
        let settings = Settings::from_db_map(&map);
        self.reload_from_settings(&settings).await
    }
}
