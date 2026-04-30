use crate::config::helpers::parse_optional_env;
use crate::error::ConfigError;

#[derive(Debug, Clone)]
pub struct BrainConfig {
    pub wm_capacity: usize,
    pub wm_margin_threshold: f64,
    pub wm_stickiness_boost: f64,
    pub wm_manual_boost: f64,
    pub wm_full_injection_threshold: f64,
    pub activation_semantic_weight: f64,
    pub activation_keyword_weight: f64,
    pub activation_neighbor_weight: f64,
    pub activation_recency_weight: f64,
    pub activation_baseline_weight: f64,
    pub neighbor_decay: f64,
    pub neighbor_depth: usize,
    pub recency_half_life_hours: f64,
    pub hebbian_delta: f64,
    pub hebbian_boost: f64,
    pub dream_decay_factor: f64,
    pub dream_interval_mins: u64,
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            wm_capacity: 12,
            wm_margin_threshold: 0.08,
            wm_stickiness_boost: 0.05,
            wm_manual_boost: 0.15,
            wm_full_injection_threshold: 0.7,
            activation_semantic_weight: 0.35,
            activation_keyword_weight: 0.25,
            activation_neighbor_weight: 0.20,
            activation_recency_weight: 0.10,
            activation_baseline_weight: 0.10,
            neighbor_decay: 0.5,
            neighbor_depth: 1,
            recency_half_life_hours: 24.0,
            hebbian_delta: 0.03,
            hebbian_boost: 0.02,
            dream_decay_factor: 0.95,
            dream_interval_mins: 30,
        }
    }
}

impl BrainConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            wm_capacity: parse_optional_env("BRAIN_WM_CAPACITY", 12usize)?,
            wm_margin_threshold: parse_optional_env("BRAIN_WM_MARGIN_THRESHOLD", 0.08f64)?,
            wm_stickiness_boost: parse_optional_env("BRAIN_WM_STICKINESS_BOOST", 0.05f64)?,
            wm_manual_boost: parse_optional_env("BRAIN_WM_MANUAL_BOOST", 0.15f64)?,
            wm_full_injection_threshold: parse_optional_env(
                "BRAIN_WM_FULL_INJECTION_THRESHOLD",
                0.7f64,
            )?,
            activation_semantic_weight: parse_optional_env(
                "BRAIN_ACTIVATION_SEMANTIC_WEIGHT",
                0.35f64,
            )?,
            activation_keyword_weight: parse_optional_env(
                "BRAIN_ACTIVATION_KEYWORD_WEIGHT",
                0.25f64,
            )?,
            activation_neighbor_weight: parse_optional_env(
                "BRAIN_ACTIVATION_NEIGHBOR_WEIGHT",
                0.20f64,
            )?,
            activation_recency_weight: parse_optional_env(
                "BRAIN_ACTIVATION_RECENCY_WEIGHT",
                0.10f64,
            )?,
            activation_baseline_weight: parse_optional_env(
                "BRAIN_ACTIVATION_BASELINE_WEIGHT",
                0.10f64,
            )?,
            neighbor_decay: parse_optional_env("BRAIN_NEIGHBOR_DECAY", 0.5f64)?,
            neighbor_depth: parse_optional_env("BRAIN_NEIGHBOR_DEPTH", 1usize)?,
            recency_half_life_hours: parse_optional_env("BRAIN_RECENCY_HALF_LIFE_HOURS", 24.0f64)?,
            hebbian_delta: parse_optional_env("BRAIN_HEBBIAN_DELTA", 0.03f64)?,
            hebbian_boost: parse_optional_env("BRAIN_HEBBIAN_BOOST", 0.02f64)?,
            dream_decay_factor: parse_optional_env("BRAIN_DREAM_DECAY_FACTOR", 0.95f64)?,
            dream_interval_mins: parse_optional_env("BRAIN_DREAM_INTERVAL_MINS", 30u64)?,
        })
    }
}
