use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::bootstrap::steward_base_dir;
use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Skills system configuration.
#[derive(Debug, Clone)]
pub struct SkillsConfig {
    /// Whether the skills system is enabled.
    pub enabled: bool,
    /// Root directory containing SKILL.md-based skills (default: ~/.steward/skills/).
    pub root_dir: PathBuf,
    /// Maximum number of skills that can be active simultaneously.
    pub max_active_skills: usize,
    /// Maximum total context tokens allocated to skill prompts.
    pub max_context_tokens: usize,
    /// Maximum recursion depth when scanning skill directories for bundle layouts.
    /// Subdirectories without `SKILL.md` are recursed into up to this depth.
    pub max_scan_depth: usize,
    /// Names of skills explicitly disabled via user settings.
    pub disabled_skills: BTreeSet<String>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            root_dir: default_skills_dir(),
            max_active_skills: 3,
            max_context_tokens: 4000,
            max_scan_depth: 3,
            disabled_skills: BTreeSet::new(),
        }
    }
}

/// Get the default user skills directory (~/.steward/skills/).
fn default_skills_dir() -> PathBuf {
    steward_base_dir().join("skills")
}

impl SkillsConfig {
    pub fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("SKILLS_ENABLED", true)?,
            root_dir: optional_env("SKILLS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_skills_dir),
            max_active_skills: parse_optional_env("SKILLS_MAX_ACTIVE", 3)?,
            max_context_tokens: parse_optional_env("SKILLS_MAX_CONTEXT_TOKENS", 4000)?,
            max_scan_depth: parse_optional_env("SKILLS_MAX_SCAN_DEPTH", 3)?,
            disabled_skills: settings
                .skills
                .disabled
                .iter()
                .map(|name| name.trim())
                .filter(|name| !name.is_empty())
                .map(|name| name.to_string())
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_uses_settings_values_when_env_is_missing() {
        let mut settings = Settings::default();
        settings.skills.disabled = vec!["officecli".to_string(), "find-skills".to_string()];

        let config = SkillsConfig::resolve(&settings).expect("resolve");

        assert!(config.enabled);
        assert_eq!(config.root_dir, steward_base_dir().join("skills"));
        assert_eq!(config.max_active_skills, 3);
        assert_eq!(config.max_context_tokens, 4000);
        assert_eq!(config.max_scan_depth, 3);
        assert!(config.disabled_skills.contains("officecli"));
        assert!(config.disabled_skills.contains("find-skills"));
    }
}
