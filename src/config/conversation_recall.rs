use crate::config::helpers::{parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::retrieval::FusionStrategy;

/// Conversation-history recall configuration resolved from environment variables.
#[derive(Debug, Clone)]
pub struct ConversationRecallConfig {
    pub fusion_strategy: FusionStrategy,
    pub rrf_k: u32,
    pub fts_weight: f32,
    pub vector_weight: f32,
    pub pre_fusion_limit: usize,
    pub seed_limit: usize,
    pub auto_prompt_base_limit: usize,
    pub auto_prompt_max_limit: usize,
    pub recent_bucket_days: i64,
    pub mid_bucket_days: i64,
    pub recent_base_quota: usize,
    pub mid_base_quota: usize,
    pub far_base_quota: usize,
    pub recent_max_quota: usize,
    pub mid_max_quota: usize,
    pub far_max_quota: usize,
    pub expand_threshold: f32,
    pub explicit_default_limit: usize,
    pub explicit_max_limit: usize,
    pub allow_group_auto_recall: bool,
}

impl Default for ConversationRecallConfig {
    fn default() -> Self {
        Self {
            fusion_strategy: FusionStrategy::Rrf,
            rrf_k: 60,
            fts_weight: 0.5,
            vector_weight: 0.5,
            pre_fusion_limit: 50,
            seed_limit: 16,
            auto_prompt_base_limit: 4,
            auto_prompt_max_limit: 7,
            recent_bucket_days: 14,
            mid_bucket_days: 90,
            recent_base_quota: 2,
            mid_base_quota: 1,
            far_base_quota: 1,
            recent_max_quota: 3,
            mid_max_quota: 2,
            far_max_quota: 2,
            expand_threshold: 0.75,
            explicit_default_limit: 8,
            explicit_max_limit: 12,
            allow_group_auto_recall: false,
        }
    }
}

impl ConversationRecallConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        let fusion_strategy = match std::env::var("CONVERSATION_RECALL_FUSION_STRATEGY")
            .ok()
            .unwrap_or_else(|| "rrf".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "rrf" => FusionStrategy::Rrf,
            "weighted" => FusionStrategy::WeightedScore,
            other => {
                return Err(ConfigError::InvalidValue {
                    key: "CONVERSATION_RECALL_FUSION_STRATEGY".to_string(),
                    message: format!("must be 'rrf' or 'weighted', got '{other}'"),
                });
            }
        };

        let (default_fts, default_vec) = match fusion_strategy {
            FusionStrategy::Rrf => (0.5f32, 0.5f32),
            FusionStrategy::WeightedScore => (0.3f32, 0.7f32),
        };
        let fts_weight = parse_optional_env("CONVERSATION_RECALL_FTS_WEIGHT", default_fts)?;
        let vector_weight = parse_optional_env("CONVERSATION_RECALL_VECTOR_WEIGHT", default_vec)?;

        if !fts_weight.is_finite() || fts_weight < 0.0 {
            return Err(ConfigError::InvalidValue {
                key: "CONVERSATION_RECALL_FTS_WEIGHT".to_string(),
                message: "must be a finite, non-negative float".to_string(),
            });
        }
        if !vector_weight.is_finite() || vector_weight < 0.0 {
            return Err(ConfigError::InvalidValue {
                key: "CONVERSATION_RECALL_VECTOR_WEIGHT".to_string(),
                message: "must be a finite, non-negative float".to_string(),
            });
        }

        let config = Self {
            fusion_strategy,
            rrf_k: parse_optional_env("CONVERSATION_RECALL_RRF_K", 60u32)?,
            fts_weight,
            vector_weight,
            pre_fusion_limit: parse_optional_env("CONVERSATION_RECALL_PRE_FUSION_LIMIT", 50usize)?,
            seed_limit: parse_optional_env("CONVERSATION_RECALL_SEED_LIMIT", 16usize)?,
            auto_prompt_base_limit: parse_optional_env(
                "CONVERSATION_RECALL_AUTO_PROMPT_BASE_LIMIT",
                4usize,
            )?,
            auto_prompt_max_limit: parse_optional_env(
                "CONVERSATION_RECALL_AUTO_PROMPT_MAX_LIMIT",
                7usize,
            )?,
            recent_bucket_days: parse_optional_env(
                "CONVERSATION_RECALL_RECENT_BUCKET_DAYS",
                14i64,
            )?,
            mid_bucket_days: parse_optional_env("CONVERSATION_RECALL_MID_BUCKET_DAYS", 90i64)?,
            recent_base_quota: parse_optional_env("CONVERSATION_RECALL_RECENT_BASE_QUOTA", 2usize)?,
            mid_base_quota: parse_optional_env("CONVERSATION_RECALL_MID_BASE_QUOTA", 1usize)?,
            far_base_quota: parse_optional_env("CONVERSATION_RECALL_FAR_BASE_QUOTA", 1usize)?,
            recent_max_quota: parse_optional_env("CONVERSATION_RECALL_RECENT_MAX_QUOTA", 3usize)?,
            mid_max_quota: parse_optional_env("CONVERSATION_RECALL_MID_MAX_QUOTA", 2usize)?,
            far_max_quota: parse_optional_env("CONVERSATION_RECALL_FAR_MAX_QUOTA", 2usize)?,
            expand_threshold: parse_optional_env("CONVERSATION_RECALL_EXPAND_THRESHOLD", 0.75f32)?,
            explicit_default_limit: parse_optional_env(
                "CONVERSATION_RECALL_EXPLICIT_DEFAULT_LIMIT",
                8usize,
            )?,
            explicit_max_limit: parse_optional_env(
                "CONVERSATION_RECALL_EXPLICIT_MAX_LIMIT",
                12usize,
            )?,
            allow_group_auto_recall: parse_bool_env(
                "CONVERSATION_RECALL_ALLOW_GROUP_AUTO_RECALL",
                false,
            )?,
        };

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.auto_prompt_base_limit == 0 {
            return Err(ConfigError::InvalidValue {
                key: "CONVERSATION_RECALL_AUTO_PROMPT_BASE_LIMIT".to_string(),
                message: "must be at least 1".to_string(),
            });
        }
        if self.auto_prompt_max_limit < self.auto_prompt_base_limit {
            return Err(ConfigError::InvalidValue {
                key: "CONVERSATION_RECALL_AUTO_PROMPT_MAX_LIMIT".to_string(),
                message: "must be >= CONVERSATION_RECALL_AUTO_PROMPT_BASE_LIMIT".to_string(),
            });
        }
        if self.mid_bucket_days < self.recent_bucket_days {
            return Err(ConfigError::InvalidValue {
                key: "CONVERSATION_RECALL_MID_BUCKET_DAYS".to_string(),
                message: "must be >= CONVERSATION_RECALL_RECENT_BUCKET_DAYS".to_string(),
            });
        }
        if !(0.0..=1.0).contains(&self.expand_threshold) {
            return Err(ConfigError::InvalidValue {
                key: "CONVERSATION_RECALL_EXPAND_THRESHOLD".to_string(),
                message: "must be between 0.0 and 1.0".to_string(),
            });
        }
        if self.explicit_max_limit < self.explicit_default_limit {
            return Err(ConfigError::InvalidValue {
                key: "CONVERSATION_RECALL_EXPLICIT_MAX_LIMIT".to_string(),
                message: "must be >= CONVERSATION_RECALL_EXPLICIT_DEFAULT_LIMIT".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    fn clear_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("CONVERSATION_RECALL_FUSION_STRATEGY");
            std::env::remove_var("CONVERSATION_RECALL_RRF_K");
            std::env::remove_var("CONVERSATION_RECALL_FTS_WEIGHT");
            std::env::remove_var("CONVERSATION_RECALL_VECTOR_WEIGHT");
            std::env::remove_var("CONVERSATION_RECALL_PRE_FUSION_LIMIT");
            std::env::remove_var("CONVERSATION_RECALL_SEED_LIMIT");
            std::env::remove_var("CONVERSATION_RECALL_AUTO_PROMPT_BASE_LIMIT");
            std::env::remove_var("CONVERSATION_RECALL_AUTO_PROMPT_MAX_LIMIT");
            std::env::remove_var("CONVERSATION_RECALL_RECENT_BUCKET_DAYS");
            std::env::remove_var("CONVERSATION_RECALL_MID_BUCKET_DAYS");
            std::env::remove_var("CONVERSATION_RECALL_RECENT_BASE_QUOTA");
            std::env::remove_var("CONVERSATION_RECALL_MID_BASE_QUOTA");
            std::env::remove_var("CONVERSATION_RECALL_FAR_BASE_QUOTA");
            std::env::remove_var("CONVERSATION_RECALL_RECENT_MAX_QUOTA");
            std::env::remove_var("CONVERSATION_RECALL_MID_MAX_QUOTA");
            std::env::remove_var("CONVERSATION_RECALL_FAR_MAX_QUOTA");
            std::env::remove_var("CONVERSATION_RECALL_EXPAND_THRESHOLD");
            std::env::remove_var("CONVERSATION_RECALL_EXPLICIT_DEFAULT_LIMIT");
            std::env::remove_var("CONVERSATION_RECALL_EXPLICIT_MAX_LIMIT");
            std::env::remove_var("CONVERSATION_RECALL_ALLOW_GROUP_AUTO_RECALL");
        }
    }

    #[test]
    fn defaults_resolve() {
        let _guard = lock_env();
        clear_env();

        let config = ConversationRecallConfig::resolve().expect("resolve");
        assert_eq!(config.auto_prompt_base_limit, 4);
        assert_eq!(config.auto_prompt_max_limit, 7);
        assert!(!config.allow_group_auto_recall);
    }

    #[test]
    fn invalid_prompt_limits_are_rejected() {
        let _guard = lock_env();
        clear_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("CONVERSATION_RECALL_AUTO_PROMPT_BASE_LIMIT", "5");
            std::env::set_var("CONVERSATION_RECALL_AUTO_PROMPT_MAX_LIMIT", "4");
        }

        let result = ConversationRecallConfig::resolve();
        assert!(result.is_err());
        clear_env();
    }
}
