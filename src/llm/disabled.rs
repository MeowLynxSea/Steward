use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::llm::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};

#[derive(Debug, Default)]
pub struct DisabledLlmProvider;

impl DisabledLlmProvider {
    pub fn new() -> Self {
        Self
    }

    fn unconfigured_error() -> LlmError {
        LlmError::RequestFailed {
            provider: "unconfigured".to_string(),
            reason: "No backend configured. Complete onboarding to choose a provider.".to_string(),
        }
    }
}

#[async_trait]
impl LlmProvider for DisabledLlmProvider {
    fn model_name(&self) -> &str {
        "unconfigured"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(Self::unconfigured_error())
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Err(Self::unconfigured_error())
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Ok(Vec::new())
    }
}
