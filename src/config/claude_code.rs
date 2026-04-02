use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env, parse_string_env};
use crate::error::ConfigError;

/// Claude Code local execution configuration.
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    /// Whether Claude Code execution is available.
    pub enabled: bool,
    /// Directory containing Claude auth config for local CLI execution.
    pub config_dir: std::path::PathBuf,
    /// Claude model to use (e.g. "sonnet", "opus").
    pub model: String,
    /// Maximum agentic turns before stopping.
    pub max_turns: u32,
    /// Soft memory budget in MB for Claude Code jobs.
    pub memory_limit_mb: u64,
    /// Allowed tool patterns for Claude Code permission settings.
    ///
    /// Written to the generated Claude settings file before spawning the CLI.
    /// Provides defense-in-depth: only explicitly listed tools are auto-approved.
    /// Any new/unknown tools would require interactive approval (which times out
    /// in non-interactive job execution, failing safely).
    ///
    /// Patterns follow Claude Code syntax: `"Bash(*)"`, `"Read"`, `"Edit(*)"`, etc.
    pub allowed_tools: Vec<String>,
}

/// Default allowed tools for Claude Code local execution.
///
/// These cover all standard Claude Code tools needed for autonomous operation.
/// Local job policy provides the primary safety boundary; this allowlist adds
/// defense-in-depth by preventing any future unknown tools from being silently
/// auto-approved.
fn default_claude_code_allowed_tools() -> Vec<String> {
    [
        // File system -- glob patterns match Claude Code's settings.json format
        "Read(*)",
        "Write(*)",
        "Edit(*)",
        "Glob(*)",
        "Grep(*)",
        "NotebookEdit(*)",
        // Execution
        "Bash(*)",
        "Task(*)",
        // Network
        "WebFetch(*)",
        "WebSearch(*)",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            config_dir: dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".claude"),
            model: "sonnet".to_string(),
            max_turns: 50,
            memory_limit_mb: 4096,
            allowed_tools: default_claude_code_allowed_tools(),
        }
    }
}

impl ClaudeCodeConfig {
    /// Load from environment variables only for minimal CLI contexts where
    /// there is no database or full config.
    pub fn from_env() -> Self {
        match Self::resolve_env_only() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to resolve ClaudeCodeConfig: {e}, using defaults");
                Self::default()
            }
        }
    }

    /// Extract the OAuth access token from the host's credential store.
    ///
    /// On macOS: reads from Keychain (`Claude Code-credentials` service).
    /// On Linux: reads from `~/.claude/.credentials.json`.
    ///
    /// Returns the access token if found. The token typically expires in
    /// 8-12 hours, which is sufficient for a single local Claude Code job.
    pub fn extract_oauth_token() -> Option<String> {
        // macOS: extract from Keychain
        if cfg!(target_os = "macos") {
            match std::process::Command::new("security")
                .args([
                    "find-generic-password",
                    "-s",
                    "Claude Code-credentials",
                    "-w",
                ])
                .output()
            {
                Ok(output) if output.status.success() => {
                    if let Ok(json) = String::from_utf8(output.stdout) {
                        return parse_oauth_access_token(json.trim());
                    }
                }
                Ok(_) => {
                    tracing::debug!("No Claude Code credentials in macOS Keychain");
                }
                Err(e) => {
                    tracing::debug!("Failed to query macOS Keychain: {e}");
                }
            }
        }

        // Linux / fallback: read from ~/.claude/.credentials.json
        if let Some(home) = dirs::home_dir() {
            let creds_path = home.join(".claude").join(".credentials.json");
            if let Ok(json) = std::fs::read_to_string(&creds_path) {
                return parse_oauth_access_token(&json);
            }
        }

        None
    }

    pub(crate) fn resolve(settings: &crate::settings::Settings) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        Ok(Self {
            enabled: parse_bool_env("CLAUDE_CODE_ENABLED", settings.claude_code.enabled)?,
            config_dir: optional_env("CLAUDE_CONFIG_DIR")?
                .map(std::path::PathBuf::from)
                .unwrap_or(defaults.config_dir),
            model: parse_string_env("CLAUDE_CODE_MODEL", defaults.model)?,
            max_turns: parse_optional_env("CLAUDE_CODE_MAX_TURNS", defaults.max_turns)?,
            memory_limit_mb: parse_optional_env(
                "CLAUDE_CODE_MEMORY_LIMIT_MB",
                defaults.memory_limit_mb,
            )?,
            allowed_tools: optional_env("CLAUDE_CODE_ALLOWED_TOOLS")?
                .map(|s| {
                    s.split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or(defaults.allowed_tools),
        })
    }

    /// Resolve from env vars only, no Settings. Used by standalone CLI paths.
    fn resolve_env_only() -> Result<Self, ConfigError> {
        let defaults = Self::default();
        Ok(Self {
            enabled: parse_bool_env("CLAUDE_CODE_ENABLED", defaults.enabled)?,
            config_dir: optional_env("CLAUDE_CONFIG_DIR")?
                .map(std::path::PathBuf::from)
                .unwrap_or(defaults.config_dir),
            model: parse_string_env("CLAUDE_CODE_MODEL", defaults.model)?,
            max_turns: parse_optional_env("CLAUDE_CODE_MAX_TURNS", defaults.max_turns)?,
            memory_limit_mb: parse_optional_env(
                "CLAUDE_CODE_MEMORY_LIMIT_MB",
                defaults.memory_limit_mb,
            )?,
            allowed_tools: optional_env("CLAUDE_CODE_ALLOWED_TOOLS")?
                .map(|s| {
                    s.split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or(defaults.allowed_tools),
        })
    }
}

/// Parse the OAuth access token from a Claude Code credentials JSON blob.
///
/// Expected shape: `{"claudeAiOauth": {"accessToken": "sk-ant-oat01-..."}}`
fn parse_oauth_access_token(json: &str) -> Option<String> {
    let creds: serde_json::Value = serde_json::from_str(json).ok()?;
    let token = creds["claudeAiOauth"]["accessToken"].as_str()?;
    // Validate that the token looks like a real OAuth token before using it.
    // Claude CLI tokens start with "sk-ant-oat".
    if !token.starts_with("sk-ant-oat") {
        tracing::debug!("Ignoring credential store token with unexpected prefix");
        return None;
    }
    Some(token.to_string())
}

#[cfg(test)]
mod tests {
    use crate::config::claude_code::*;
    use crate::testing::credentials::*;

    #[test]
    fn claude_code_config_default_values() {
        let cfg = ClaudeCodeConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.model, "sonnet");
        assert_eq!(cfg.max_turns, 50);
        assert_eq!(cfg.memory_limit_mb, 4096);
        assert!(cfg.config_dir.ends_with(".claude"));
        // Should have all the standard tools
        assert!(!cfg.allowed_tools.is_empty());
        assert!(cfg.allowed_tools.contains(&"Bash(*)".to_string()));
        assert!(cfg.allowed_tools.contains(&"Read(*)".to_string()));
        assert!(cfg.allowed_tools.contains(&"Edit(*)".to_string()));
        assert!(cfg.allowed_tools.contains(&"Write(*)".to_string()));
        assert!(cfg.allowed_tools.contains(&"Grep(*)".to_string()));
        assert!(cfg.allowed_tools.contains(&"WebFetch(*)".to_string()));
    }

    #[test]
    fn claude_code_config_custom_values() {
        let cfg = ClaudeCodeConfig {
            enabled: true,
            config_dir: std::path::PathBuf::from("/opt/claude"),
            model: "opus".to_string(),
            max_turns: 100,
            memory_limit_mb: 8192,
            allowed_tools: vec!["Read(*)".to_string(), "Bash(*)".to_string()],
        };
        assert!(cfg.enabled);
        assert_eq!(cfg.config_dir, std::path::PathBuf::from("/opt/claude"));
        assert_eq!(cfg.model, "opus");
        assert_eq!(cfg.max_turns, 100);
        assert_eq!(cfg.memory_limit_mb, 8192);
        assert_eq!(cfg.allowed_tools.len(), 2);
    }

    #[test]
    fn parse_oauth_token_valid() {
        let json = format!(
            r#"{{"claudeAiOauth": {{"accessToken": "{}"}}}}"#,
            TEST_ANTHROPIC_OAUTH_BASIC
        );
        let token = parse_oauth_access_token(&json);
        assert_eq!(token, Some(TEST_ANTHROPIC_OAUTH_BASIC.to_string()));
    }

    #[test]
    fn parse_oauth_token_missing_access_token() {
        let json = r#"{"claudeAiOauth": {}}"#;
        assert_eq!(parse_oauth_access_token(json), None);
    }

    #[test]
    fn parse_oauth_token_missing_oauth_key() {
        let json = r#"{"someOtherKey": {"accessToken": "tok"}}"#;
        assert_eq!(parse_oauth_access_token(json), None);
    }

    #[test]
    fn parse_oauth_token_invalid_json() {
        assert_eq!(parse_oauth_access_token("not json at all"), None);
    }

    #[test]
    fn parse_oauth_token_empty_string() {
        assert_eq!(parse_oauth_access_token(""), None);
    }

    #[test]
    fn parse_oauth_token_nested_extra_fields() {
        let json = format!(
            r#"{{
            "claudeAiOauth": {{
                "accessToken": "{}",
                "refreshToken": "rt-abc",
                "expiresAt": 1700000000
            }}
        }}"#,
            TEST_ANTHROPIC_OAUTH_NESTED
        );
        assert_eq!(
            parse_oauth_access_token(&json),
            Some(TEST_ANTHROPIC_OAUTH_NESTED.to_string())
        );
    }

    #[test]
    fn parse_oauth_token_access_token_is_not_string() {
        let json = r#"{"claudeAiOauth": {"accessToken": 12345}}"#;
        assert_eq!(parse_oauth_access_token(json), None);
    }

    #[test]
    fn parse_oauth_token_rejects_invalid_prefix() {
        let json = r#"{"claudeAiOauth": {"accessToken": "not-an-oauth-token"}}"#;
        assert_eq!(parse_oauth_access_token(json), None);
    }

    #[test]
    fn default_allowed_tools_has_expected_count() {
        let tools = default_claude_code_allowed_tools();
        // 10 tools: Read, Write, Edit, Glob, Grep, NotebookEdit, Bash, Task, WebFetch, WebSearch
        assert_eq!(tools.len(), 10);
    }

    #[test]
    fn default_allowed_tools_all_have_glob_pattern() {
        let tools = default_claude_code_allowed_tools();
        for tool in &tools {
            assert!(
                tool.ends_with("(*)"),
                "tool '{tool}' should end with '(*)' glob pattern"
            );
        }
    }

    #[test]
    fn claude_code_resolve_uses_settings_enabled() {
        let _guard = crate::config::helpers::lock_env();
        let mut settings = crate::settings::Settings::default();
        settings.claude_code.enabled = true;

        let cfg = ClaudeCodeConfig::resolve(&settings).expect("resolve");
        assert!(cfg.enabled);
    }

    #[test]
    fn claude_code_resolve_defaults_disabled() {
        let _guard = crate::config::helpers::lock_env();
        let settings = crate::settings::Settings::default();
        let cfg = ClaudeCodeConfig::resolve(&settings).expect("resolve");
        assert!(!cfg.enabled);
    }

    #[test]
    fn claude_code_env_overrides_settings() {
        let _guard = crate::config::helpers::lock_env();
        let mut settings = crate::settings::Settings::default();
        settings.claude_code.enabled = true;

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("CLAUDE_CODE_ENABLED", "false") };
        let cfg = ClaudeCodeConfig::resolve(&settings).expect("resolve");
        unsafe { std::env::remove_var("CLAUDE_CODE_ENABLED") };

        assert!(!cfg.enabled);
    }
}
