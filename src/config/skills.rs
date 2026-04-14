use std::path::PathBuf;

use crate::bootstrap::steward_base_dir;
use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

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
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            root_dir: default_skills_dir(),
            max_active_skills: 3,
            max_context_tokens: 4000,
            max_scan_depth: 3,
        }
    }
}

/// Get the default user skills directory (~/.steward/skills/).
fn default_skills_dir() -> PathBuf {
    steward_base_dir().join("skills")
}

impl SkillsConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("SKILLS_ENABLED", true)?,
            root_dir: optional_env("SKILLS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_skills_dir),
            max_active_skills: parse_optional_env("SKILLS_MAX_ACTIVE", 3)?,
            max_context_tokens: parse_optional_env("SKILLS_MAX_CONTEXT_TOKENS", 4000)?,
            max_scan_depth: parse_optional_env("SKILLS_MAX_SCAN_DEPTH", 3)?,
        })
    }
}
