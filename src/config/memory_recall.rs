use crate::config::helpers::parse_optional_env;
use crate::error::ConfigError;
use crate::retrieval::FusionStrategy;

#[derive(Debug, Clone)]
pub struct MemoryRecallConfig {
    pub fusion_strategy: FusionStrategy,
    pub rrf_k: u32,
    pub fts_weight: f32,
    pub vector_weight: f32,
    pub pre_fusion_limit: usize,
    pub boot_limit: usize,
    pub trigger_limit: usize,
    pub seed_limit: usize,
    pub expansion_limit: usize,
    pub recent_limit: usize,
}

impl Default for MemoryRecallConfig {
    fn default() -> Self {
        Self {
            fusion_strategy: FusionStrategy::Rrf,
            rrf_k: 60,
            fts_weight: 0.5,
            vector_weight: 0.5,
            pre_fusion_limit: 50,
            boot_limit: 8,
            trigger_limit: 6,
            seed_limit: 8,
            expansion_limit: 6,
            recent_limit: 3,
        }
    }
}

impl MemoryRecallConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        let fusion_strategy = match std::env::var("MEMORY_RECALL_FUSION_STRATEGY")
            .ok()
            .unwrap_or_else(|| "rrf".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "rrf" => FusionStrategy::Rrf,
            "weighted" => FusionStrategy::WeightedScore,
            other => {
                return Err(ConfigError::InvalidValue {
                    key: "MEMORY_RECALL_FUSION_STRATEGY".to_string(),
                    message: format!("must be 'rrf' or 'weighted', got '{other}'"),
                });
            }
        };

        let (default_fts, default_vec) = match fusion_strategy {
            FusionStrategy::Rrf => (0.5f32, 0.5f32),
            FusionStrategy::WeightedScore => (0.3f32, 0.7f32),
        };
        let fts_weight = parse_optional_env("MEMORY_RECALL_FTS_WEIGHT", default_fts)?;
        let vector_weight = parse_optional_env("MEMORY_RECALL_VECTOR_WEIGHT", default_vec)?;
        if !fts_weight.is_finite() || fts_weight < 0.0 {
            return Err(ConfigError::InvalidValue {
                key: "MEMORY_RECALL_FTS_WEIGHT".to_string(),
                message: "must be a finite, non-negative float".to_string(),
            });
        }
        if !vector_weight.is_finite() || vector_weight < 0.0 {
            return Err(ConfigError::InvalidValue {
                key: "MEMORY_RECALL_VECTOR_WEIGHT".to_string(),
                message: "must be a finite, non-negative float".to_string(),
            });
        }

        Ok(Self {
            fusion_strategy,
            rrf_k: parse_optional_env("MEMORY_RECALL_RRF_K", 60u32)?,
            fts_weight,
            vector_weight,
            pre_fusion_limit: parse_optional_env("MEMORY_RECALL_PRE_FUSION_LIMIT", 50usize)?,
            boot_limit: parse_optional_env("MEMORY_RECALL_BOOT_LIMIT", 8usize)?,
            trigger_limit: parse_optional_env("MEMORY_RECALL_TRIGGER_LIMIT", 6usize)?,
            seed_limit: parse_optional_env("MEMORY_RECALL_SEED_LIMIT", 8usize)?,
            expansion_limit: parse_optional_env("MEMORY_RECALL_EXPANSION_LIMIT", 6usize)?,
            recent_limit: parse_optional_env("MEMORY_RECALL_RECENT_LIMIT", 3usize)?,
        })
    }
}
