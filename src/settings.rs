//! User settings persistence.
//!
//! Stores user preferences in `~/.steward` (JSON/TOML) and in the database.
//! The LLM configuration is represented only by the current multi-backend
//! model: a list of configured backends plus the selected major/cheap IDs.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::bootstrap::steward_base_dir;

/// Canonical secret name for a backend provider's API key.
pub fn builtin_secret_name(provider_id: &str) -> String {
    format!("llm_builtin_{provider_id}_api_key")
}

/// A configured LLM backend instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendInstance {
    /// Unique identifier (UUID).
    pub id: String,
    /// Provider ID.
    pub provider: String,
    /// API key override for this backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Base URL override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Model identifier. Empty string means "use provider default".
    #[serde(default)]
    pub model: String,
    /// OpenAI-only request format (`chat_completions` or `responses`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_format: Option<String>,
    /// Manually specified context window size in tokens.
    /// When `None`, the value is fetched from the provider's model metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
}

/// User settings persisted to disk and DB.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    /// Whether onboarding wizard has been completed.
    #[serde(default, alias = "setup_completed")]
    pub onboard_completed: bool,

    /// Stable owner scope for this Steward instance.
    #[serde(default)]
    pub owner_id: Option<String>,

    #[serde(default)]
    pub libsql_path: Option<String>,

    #[serde(default)]
    pub libsql_url: Option<String>,

    #[serde(default)]
    pub secrets_master_key_source: KeySource,

    #[serde(default, skip_serializing)]
    pub secrets_master_key_hex: Option<String>,

    /// All configured backend instances.
    #[serde(default)]
    pub backends: Vec<BackendInstance>,

    /// ID of the backend instance used for the major (primary) model.
    #[serde(default)]
    pub major_backend_id: Option<String>,

    /// ID of the backend instance used for the cheap model.
    #[serde(default)]
    pub cheap_backend_id: Option<String>,

    /// When true, reuse the major model instead of a separate cheap model.
    #[serde(default = "default_true")]
    pub cheap_model_uses_primary: bool,

    #[serde(default)]
    pub embeddings: EmbeddingsSettings,

    #[serde(default)]
    pub skills: SkillsSettings,

    #[serde(default)]
    pub tunnel: TunnelSettings,

    #[serde(default)]
    pub channels: ChannelSettings,

    #[serde(default)]
    pub heartbeat: HeartbeatSettings,

    /// Whether the user completed the initial bootstrap/onboarding flow.
    ///
    /// Backward-compatible aliases:
    /// - `profile_onboarding_completed` (legacy psychographic profile onboarding)
    /// - `personal_onboarding_completed` (older name)
    #[serde(
        default,
        alias = "profile_onboarding_completed",
        alias = "personal_onboarding_completed"
    )]
    pub bootstrap_onboarding_completed: bool,

    #[serde(default)]
    pub agent: AgentSettings,

    #[serde(default)]
    pub wasm: WasmSettings,

    #[serde(default)]
    pub claude_code: ClaudeCodeSettings,

    #[serde(default)]
    pub safety: SafetySettings,

    #[serde(default)]
    pub builder: BuilderSettings,

    #[serde(default)]
    pub transcription: Option<TranscriptionSettings>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    Keychain,
    Env,
    #[default]
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsSettings {
    #[serde(default)]
    pub enabled: bool,
    /// Provider to use: "openai" or "ollama".
    #[serde(default = "default_embeddings_provider")]
    pub provider: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_embeddings_model")]
    pub model: String,
    #[serde(default)]
    pub dimension: Option<usize>,
}

fn default_embeddings_provider() -> String {
    "openai".to_string()
}

fn default_embeddings_model() -> String {
    "text-embedding-3-small".to_string()
}

impl Default for EmbeddingsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_embeddings_provider(),
            api_key: None,
            base_url: None,
            model: default_embeddings_model(),
            dimension: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsSettings {
    #[serde(default)]
    pub disabled: Vec<String>,
}

impl Default for SkillsSettings {
    fn default() -> Self {
        Self {
            disabled: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TunnelSettings {
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub cf_token: Option<String>,
    #[serde(default)]
    pub ngrok_token: Option<String>,
    #[serde(default)]
    pub ngrok_domain: Option<String>,
    #[serde(default)]
    pub ts_funnel: bool,
    #[serde(default)]
    pub ts_hostname: Option<String>,
    #[serde(default)]
    pub custom_command: Option<String>,
    #[serde(default)]
    pub custom_health_url: Option<String>,
    #[serde(default)]
    pub custom_url_pattern: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSettings {
    #[serde(default = "default_true")]
    pub tauri_ipc: bool,
    #[serde(default)]
    pub wasm_channels_enabled: bool,
    #[serde(default)]
    pub wasm_channels_dir: Option<PathBuf>,
}

impl Default for ChannelSettings {
    fn default() -> Self {
        Self {
            tauri_ipc: true,
            wasm_channels_enabled: false,
            wasm_channels_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_heartbeat_interval")]
    pub interval_secs: u64,
    #[serde(default)]
    pub notify_channel: Option<String>,
    #[serde(default)]
    pub notify_user: Option<String>,
    #[serde(default)]
    pub fire_at: Option<String>,
    #[serde(default)]
    pub quiet_hours_start: Option<u32>,
    #[serde(default)]
    pub quiet_hours_end: Option<u32>,
    #[serde(default)]
    pub timezone: Option<String>,
}

fn default_heartbeat_interval() -> u64 {
    1800
}

impl Default for HeartbeatSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: default_heartbeat_interval(),
            notify_channel: None,
            notify_user: None,
            fire_at: None,
            quiet_hours_start: None,
            quiet_hours_end: None,
            timezone: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSettings {
    #[serde(default = "default_agent_name")]
    pub name: String,
    #[serde(default = "default_max_parallel_jobs")]
    pub max_parallel_jobs: u32,
    #[serde(default = "default_job_timeout")]
    pub job_timeout_secs: u64,
    #[serde(default = "default_stuck_threshold")]
    pub stuck_threshold_secs: u64,
    #[serde(default = "default_true")]
    pub use_planning: bool,
    #[serde(default = "default_repair_interval")]
    pub repair_check_interval_secs: u64,
    #[serde(default = "default_max_repair_attempts")]
    pub max_repair_attempts: u32,
    #[serde(default = "default_max_tool_iterations")]
    pub max_tool_iterations: usize,
    #[serde(default)]
    pub auto_approve_tools: bool,
    #[serde(default = "default_timezone")]
    pub default_timezone: String,
    #[serde(default)]
    pub max_tokens_per_job: u64,
    #[serde(default)]
    pub max_llm_concurrent_per_user: Option<usize>,
    #[serde(default)]
    pub max_jobs_concurrent_per_user: Option<usize>,
}

fn default_agent_name() -> String {
    "steward".to_string()
}

fn default_max_parallel_jobs() -> u32 {
    5
}

fn default_job_timeout() -> u64 {
    3600
}

fn default_stuck_threshold() -> u64 {
    300
}

fn default_repair_interval() -> u64 {
    60
}

fn default_max_repair_attempts() -> u32 {
    3
}

fn default_max_tool_iterations() -> usize {
    50
}

fn default_timezone() -> String {
    "UTC".to_string()
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            name: default_agent_name(),
            max_parallel_jobs: default_max_parallel_jobs(),
            job_timeout_secs: default_job_timeout(),
            stuck_threshold_secs: default_stuck_threshold(),
            use_planning: true,
            repair_check_interval_secs: default_repair_interval(),
            max_repair_attempts: default_max_repair_attempts(),
            max_tool_iterations: default_max_tool_iterations(),
            auto_approve_tools: false,
            default_timezone: default_timezone(),
            max_tokens_per_job: 0,
            max_llm_concurrent_per_user: None,
            max_jobs_concurrent_per_user: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub tools_dir: Option<PathBuf>,
    #[serde(default = "default_wasm_memory_limit")]
    pub default_memory_limit: u64,
    #[serde(default = "default_wasm_timeout")]
    pub default_timeout_secs: u64,
    #[serde(default = "default_wasm_fuel_limit")]
    pub default_fuel_limit: u64,
    #[serde(default = "default_true")]
    pub cache_compiled: bool,
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
}

fn default_wasm_memory_limit() -> u64 {
    10 * 1024 * 1024
}

fn default_wasm_timeout() -> u64 {
    60
}

fn default_wasm_fuel_limit() -> u64 {
    10_000_000
}

impl Default for WasmSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            tools_dir: None,
            default_memory_limit: default_wasm_memory_limit(),
            default_timeout_secs: default_wasm_timeout(),
            default_fuel_limit: default_wasm_fuel_limit(),
            cache_compiled: true,
            cache_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaudeCodeSettings {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetySettings {
    #[serde(default = "default_max_output_length")]
    pub max_output_length: usize,
    #[serde(default = "default_true")]
    pub injection_check_enabled: bool,
}

fn default_max_output_length() -> usize {
    100_000
}

impl Default for SafetySettings {
    fn default() -> Self {
        Self {
            max_output_length: default_max_output_length(),
            injection_check_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuilderSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub build_dir: Option<PathBuf>,
    #[serde(default = "default_builder_max_iterations")]
    pub max_iterations: u32,
    #[serde(default = "default_builder_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_true")]
    pub auto_register: bool,
}

fn default_builder_max_iterations() -> u32 {
    20
}

fn default_builder_timeout() -> u64 {
    600
}

impl Default for BuilderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            build_dir: None,
            max_iterations: default_builder_max_iterations(),
            timeout_secs: default_builder_timeout(),
            auto_register: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionSettings {
    #[serde(default)]
    pub enabled: bool,
}

impl Settings {
    pub fn get_backend(&self, id: &str) -> Option<&BackendInstance> {
        self.backends.iter().find(|b| b.id == id)
    }

    pub fn major_backend(&self) -> Option<&BackendInstance> {
        self.major_backend_id
            .as_ref()
            .and_then(|id| self.get_backend(id))
            .or_else(|| self.backends.first())
    }

    pub fn cheap_backend(&self) -> Option<&BackendInstance> {
        if self.cheap_model_uses_primary {
            return self.major_backend();
        }
        self.cheap_backend_id
            .as_ref()
            .and_then(|id| self.get_backend(id))
    }

    pub fn from_db_map(map: &std::collections::HashMap<String, serde_json::Value>) -> Self {
        let mut root = serde_json::Map::new();
        for (key, value) in map {
            if key == "owner_id" || value.is_null() {
                continue;
            }
            insert_db_value(&mut root, key, value.clone());
        }

        serde_json::from_value(serde_json::Value::Object(root)).unwrap_or_else(|e| {
            tracing::warn!("Failed to deserialize DB settings map: {}", e);
            Self::default()
        })
    }

    pub fn to_db_map(&self) -> std::collections::HashMap<String, serde_json::Value> {
        let json = match serde_json::to_value(self) {
            Ok(v) => v,
            Err(_) => return std::collections::HashMap::new(),
        };

        let mut map = std::collections::HashMap::new();
        collect_settings_json(&json, String::new(), &mut map);
        map.remove("owner_id");
        map
    }

    pub fn default_path() -> std::path::PathBuf {
        steward_base_dir().join("settings.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::default_path())
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn default_toml_path() -> PathBuf {
        steward_base_dir().join("config.toml")
    }

    pub fn load_toml(path: &std::path::Path) -> Result<Option<Self>, String> {
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("failed to read {}: {}", path.display(), e)),
        };

        let settings: Self = toml::from_str(&data)
            .map_err(|e| format!("invalid TOML in {}: {}", path.display(), e))?;
        Ok(Some(settings))
    }

    pub fn save_toml(&self, path: &std::path::Path) -> Result<(), String> {
        let raw = toml::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize settings: {}", e))?;

        let content = format!(
            "# Steward configuration file.\n\
             #\n\
             # LLM settings are represented only by configured `backends`.\n\
             # Add one or more backends and choose `major_backend_id`.\n\
             \n\
             {raw}"
        );

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
        }

        std::fs::write(path, content)
            .map_err(|e| format!("failed to write {}: {}", path.display(), e))
    }

    pub fn merge_from(&mut self, other: &Self) {
        let default_json = match serde_json::to_value(Self::default()) {
            Ok(v) => v,
            Err(_) => return,
        };
        let other_json = match serde_json::to_value(other) {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut self_json = match serde_json::to_value(&*self) {
            Ok(v) => v,
            Err(_) => return,
        };

        merge_non_default(&mut self_json, &other_json, &default_json);

        if let Ok(merged) = serde_json::from_value(self_json) {
            *self = merged;
        }
    }

    pub fn get(&self, path: &str) -> Option<String> {
        let json = serde_json::to_value(self).ok()?;
        let mut current = &json;

        for part in path.split('.') {
            current = current.get(part)?;
        }

        match current {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            serde_json::Value::Null => Some("null".to_string()),
            serde_json::Value::Array(arr) => Some(serde_json::to_string(arr).unwrap_or_default()),
            serde_json::Value::Object(obj) => Some(serde_json::to_string(obj).unwrap_or_default()),
        }
    }

    pub fn set(&mut self, path: &str, value: &str) -> Result<(), String> {
        let mut json = serde_json::to_value(&self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        let parts: Vec<&str> = path.split('.').collect();
        let (final_key, parent_parts) =
            parts.split_last().ok_or_else(|| "Empty path".to_string())?;

        let mut current = &mut json;
        for part in parent_parts {
            current = current
                .get_mut(*part)
                .ok_or_else(|| format!("Path not found: {}", path))?;
        }
        let obj = current
            .as_object_mut()
            .ok_or_else(|| format!("Parent is not an object: {}", path))?;

        let new_value = if let Some(existing) = obj.get(*final_key) {
            match existing {
                serde_json::Value::Bool(_) => {
                    let b = value
                        .parse::<bool>()
                        .map_err(|_| format!("Expected boolean for {}, got '{}'", path, value))?;
                    serde_json::Value::Bool(b)
                }
                serde_json::Value::Number(n) => {
                    if n.is_u64() {
                        let n = value.parse::<u64>().map_err(|_| {
                            format!("Expected integer for {}, got '{}'", path, value)
                        })?;
                        serde_json::Value::Number(n.into())
                    } else if n.is_i64() {
                        let n = value.parse::<i64>().map_err(|_| {
                            format!("Expected integer for {}, got '{}'", path, value)
                        })?;
                        serde_json::Value::Number(n.into())
                    } else {
                        let n = value.parse::<f64>().map_err(|_| {
                            format!("Expected number for {}, got '{}'", path, value)
                        })?;
                        serde_json::Number::from_f64(n)
                            .map(serde_json::Value::Number)
                            .unwrap_or(serde_json::Value::String(value.to_string()))
                    }
                }
                serde_json::Value::Null => serde_json::from_str(value)
                    .unwrap_or(serde_json::Value::String(value.to_string())),
                serde_json::Value::Array(_) => serde_json::from_str(value)
                    .map_err(|e| format!("Invalid JSON array for {}: {}", path, e))?,
                serde_json::Value::Object(_) => serde_json::from_str(value)
                    .map_err(|e| format!("Invalid JSON object for {}: {}", path, e))?,
                serde_json::Value::String(_) => serde_json::Value::String(value.to_string()),
            }
        } else {
            serde_json::from_str(value).unwrap_or(serde_json::Value::String(value.to_string()))
        };

        obj.insert((*final_key).to_string(), new_value);

        *self =
            serde_json::from_value(json).map_err(|e| format!("Failed to apply setting: {}", e))?;
        Ok(())
    }

    pub fn reset(&mut self, path: &str) -> Result<(), String> {
        let default = Self::default();
        let default_value = default
            .get(path)
            .ok_or_else(|| format!("Unknown setting: {}", path))?;

        self.set(path, &default_value)
    }

    pub fn list(&self) -> Vec<(String, String)> {
        let json = match serde_json::to_value(self) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();
        collect_settings(&json, String::new(), &mut results);
        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }
}

fn collect_settings_json(
    value: &serde_json::Value,
    prefix: String,
    results: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            for (key, val) in obj {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                collect_settings_json(val, path, results);
            }
        }
        other => {
            results.insert(prefix, other.clone());
        }
    }
}

fn insert_db_value(
    root: &mut serde_json::Map<String, serde_json::Value>,
    path: &str,
    value: serde_json::Value,
) {
    let mut parts = path.split('.').peekable();
    let mut current = root;

    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            current.insert(part.to_string(), value);
            return;
        }

        let entry = current
            .entry(part.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if !entry.is_object() {
            *entry = serde_json::Value::Object(serde_json::Map::new());
        }
        current = entry
            .as_object_mut()
            .expect("object entry should stay object while inserting db value");
    }
}

fn collect_settings(
    value: &serde_json::Value,
    prefix: String,
    results: &mut Vec<(String, String)>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            for (key, val) in obj {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                collect_settings(val, path, results);
            }
        }
        serde_json::Value::Array(arr) => {
            results.push((prefix, serde_json::to_string(arr).unwrap_or_default()));
        }
        serde_json::Value::String(s) => results.push((prefix, s.clone())),
        serde_json::Value::Number(n) => results.push((prefix, n.to_string())),
        serde_json::Value::Bool(b) => results.push((prefix, b.to_string())),
        serde_json::Value::Null => results.push((prefix, "null".to_string())),
    }
}

fn merge_non_default(
    target: &mut serde_json::Value,
    other: &serde_json::Value,
    defaults: &serde_json::Value,
) {
    match (target, other, defaults) {
        (
            serde_json::Value::Object(t),
            serde_json::Value::Object(o),
            serde_json::Value::Object(d),
        ) => {
            for (key, other_val) in o {
                let default_val = d.get(key).cloned().unwrap_or(serde_json::Value::Null);
                if let Some(target_val) = t.get_mut(key) {
                    merge_non_default(target_val, other_val, &default_val);
                } else if other_val != &default_val {
                    t.insert(key.clone(), other_val.clone());
                }
            }
        }
        (target, other, defaults) => {
            if other != defaults {
                *target = other.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_backend_falls_back_to_first_backend() {
        let settings = Settings {
            backends: vec![BackendInstance {
                id: "b1".to_string(),
                provider: "openai".to_string(),
                api_key: None,
                base_url: None,
                model: "gpt-5-mini".to_string(),
                request_format: Some("chat_completions".to_string()),
                context_length: None,
            }],
            ..Default::default()
        };

        assert_eq!(
            settings.major_backend().map(|backend| backend.id.as_str()),
            Some("b1")
        );
    }

    #[test]
    fn embeddings_default_to_openai() {
        let settings = Settings::default();
        assert_eq!(settings.embeddings.provider, "openai");
        assert_eq!(settings.embeddings.model, "text-embedding-3-small");
        assert_eq!(settings.embeddings.dimension, None);
    }

    #[test]
    fn skills_default_to_empty_disabled_list() {
        let settings = Settings::default();
        assert!(settings.skills.disabled.is_empty());
    }

    #[test]
    fn merge_from_preserves_non_default_backend_settings() {
        let mut base = Settings::default();
        let overlay = Settings {
            backends: vec![BackendInstance {
                id: "major".to_string(),
                provider: "anthropic".to_string(),
                api_key: Some("test".to_string()),
                base_url: Some("https://api.anthropic.com".to_string()),
                model: "claude-sonnet-4-20250514".to_string(),
                request_format: None,
                context_length: None,
            }],
            major_backend_id: Some("major".to_string()),
            ..Default::default()
        };

        base.merge_from(&overlay);

        assert_eq!(base.backends.len(), 1);
        assert_eq!(base.major_backend_id.as_deref(), Some("major"));
    }
}
