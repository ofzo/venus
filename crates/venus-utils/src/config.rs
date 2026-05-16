use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::debug;

use crate::fs_helpers;

/// Top-level Venus configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    /// Active provider name (key into `[provider]` map).
    #[serde(default)]
    pub active_provider: Option<String>,

    /// Model to use (overrides provider's default_model).
    #[serde(default)]
    pub model: Option<String>,

    /// Max output tokens.
    #[serde(default)]
    pub max_tokens: Option<u32>,

    /// Permission mode: default, auto, bypass.
    #[serde(default)]
    pub permission_mode: Option<String>,

    /// Extra system prompt appended to the default.
    #[serde(default)]
    pub custom_system_prompt: Option<String>,

    /// Max agentic turns per query.
    #[serde(default)]
    pub max_turns: Option<u32>,

    /// Max budget in USD.
    #[serde(default)]
    pub budget_usd: Option<f64>,

    /// Allowed tools (if set, only these are permitted).
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,

    /// Disallowed tools.
    #[serde(default)]
    pub disallowed_tools: Option<Vec<String>>,

    /// Extended thinking configuration.
    #[serde(default)]
    pub thinking: Option<ThinkingConfig>,

    /// Permission allow rules.
    #[serde(default)]
    pub always_allow: Option<Vec<PermissionRule>>,

    /// Permission deny rules.
    #[serde(default)]
    pub always_deny: Option<Vec<PermissionRule>>,

    /// Provider configurations (name -> config).
    #[serde(default)]
    pub provider: Option<HashMap<String, ProviderConfig>>,

    /// MCP server configurations (name -> config).
    #[serde(default)]
    pub mcp_servers: Option<HashMap<String, McpServerConfig>>,

    /// Hook configurations (event name -> list of hook entries).
    #[serde(default)]
    pub hooks: Option<HookConfig>,
}

/// Configuration for an API provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Provider type: "anthropic", "openai", "openai-compatible".
    #[serde(rename = "type")]
    pub provider_type: String,

    /// API key for this provider (also accepts "key" as alias).
    #[serde(default, alias = "key")]
    pub api_key: Option<String>,

    /// OAuth Bearer token (alternative to api_key, takes precedence).
    #[serde(default)]
    pub auth_token: Option<String>,

    /// API base URL.
    #[serde(default)]
    pub base_url: Option<String>,

    /// Default model for this provider (also accepts "model" as alias).
    #[serde(default, alias = "model")]
    pub default_model: Option<String>,

    /// Default max tokens for this provider.
    #[serde(default)]
    pub max_tokens: Option<u32>,

    /// API version header (for Anthropic-compatible APIs).
    #[serde(default)]
    pub api_version: Option<String>,
}

/// Extended thinking configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    /// Thinking mode: "adaptive", "enabled", or "disabled".
    #[serde(default)]
    pub mode: Option<String>,
    /// Budget tokens for "enabled" mode. Ignored for "adaptive".
    #[serde(default)]
    pub budget_tokens: Option<u32>,
}

/// Hook configuration: maps event names to lists of hook entries.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookConfig {
    #[serde(flatten)]
    pub entries: HashMap<String, Vec<HookEntry>>,
}

/// A single hook entry with an optional matcher and a list of hooks to execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEntry {
    /// Tool name pattern for tool events (pipe-separated: "Edit|Write|Bash").
    #[serde(default)]
    pub matcher: Option<String>,
    /// The hook definitions to execute.
    #[serde(default)]
    pub hooks: Vec<CommandHook>,
}

/// A shell command to execute as a hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHook {
    /// Hook type: "command" (default) or "http".
    #[serde(default, rename = "type")]
    pub hook_type: Option<String>,
    /// Shell command string or URL for HTTP hooks.
    pub command: String,
    /// Timeout in seconds (default: 10).
    #[serde(default = "default_hook_timeout")]
    pub timeout: u64,
    /// Conditional expression.
    #[serde(default, rename = "if")]
    pub r#if: Option<String>,
    /// Only run once per session.
    #[serde(default)]
    pub once: bool,
    /// Run asynchronously (non-blocking).
    #[serde(default)]
    pub r#async: bool,
    /// Custom HTTP headers for "http" type hooks.
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
}

fn default_hook_timeout() -> u64 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub tool: String,
    #[serde(default)]
    pub pattern: Option<String>,
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Command to launch the MCP server.
    pub command: String,
    /// Arguments for the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables for the server process.
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    /// Transport type: "stdio" (default) or "sse".
    #[serde(default = "default_mcp_transport")]
    pub transport: String,
    /// URL for SSE transport.
    #[serde(default)]
    pub url: Option<String>,
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

impl Settings {
    /// Load settings from global config only.
    pub fn load() -> Result<Self> {
        Self::load_with_project(None)
    }

    /// Load settings with multi-level merging.
    /// Merge order: defaults → ~/.venus/config.toml → project .venus/config.toml
    pub fn load_with_project(project_root: Option<&std::path::Path>) -> Result<Self> {
        let mut settings = Settings::default();

        // 1. Global settings (~/.venus/config.toml)
        if let Ok(config_dir) = fs_helpers::venus_config_dir() {
            let settings_path = config_dir.join("config.toml");
            if settings_path.exists() {
                let content = std::fs::read_to_string(&settings_path)?;
                let file_settings: Settings = toml::from_str(&content)?;
                settings.merge(file_settings);
                debug!("loaded global settings from {:?}", settings_path);
            }
        }

        // 2. Project settings (.venus/config.toml)
        if let Some(root) = project_root {
            let project_path = root.join(".venus").join("config.toml");
            if project_path.exists() {
                let content = std::fs::read_to_string(&project_path)?;
                let file_settings: Settings = toml::from_str(&content)?;
                settings.merge(file_settings);
                debug!("loaded project settings from {:?}", project_path);
            }
        }

        Ok(settings)
    }

    fn merge(&mut self, other: Settings) {
        if other.active_provider.is_some() {
            self.active_provider = other.active_provider;
        }
        if other.model.is_some() {
            self.model = other.model;
        }
        if other.max_tokens.is_some() {
            self.max_tokens = other.max_tokens;
        }
        if other.permission_mode.is_some() {
            self.permission_mode = other.permission_mode;
        }
        if other.custom_system_prompt.is_some() {
            self.custom_system_prompt = other.custom_system_prompt;
        }
        if other.max_turns.is_some() {
            self.max_turns = other.max_turns;
        }
        if other.budget_usd.is_some() {
            self.budget_usd = other.budget_usd;
        }
        if other.allowed_tools.is_some() {
            self.allowed_tools = other.allowed_tools;
        }
        if other.disallowed_tools.is_some() {
            self.disallowed_tools = other.disallowed_tools;
        }
        if other.thinking.is_some() {
            self.thinking = other.thinking;
        }
        if other.always_allow.is_some() {
            self.always_allow = other.always_allow;
        }
        if other.always_deny.is_some() {
            self.always_deny = other.always_deny;
        }
        if let Some(other_providers) = other.provider {
            self.provider
                .get_or_insert_with(HashMap::new)
                .extend(other_providers);
        }
        if other.mcp_servers.is_some() {
            self.mcp_servers = other.mcp_servers;
        }
        if other.hooks.is_some() {
            self.hooks = other.hooks;
        }
    }

    /// Get the active provider config.
    pub fn active_provider_config(&self) -> Option<&ProviderConfig> {
        let name = self.active_provider.as_deref().unwrap_or("anthropic");
        self.provider.as_ref()?.get(name)
    }

    /// Get the effective model name.
    pub fn effective_model(&self) -> &str {
        if let Some(ref m) = self.model {
            return m;
        }
        if let Some(p) = self.active_provider_config() {
            if let Some(ref m) = p.default_model {
                return m;
            }
        }
        "claude-sonnet-4-20250514"
    }

    /// Get the effective max tokens.
    pub fn effective_max_tokens(&self) -> u32 {
        if let Some(t) = self.max_tokens {
            return t;
        }
        if let Some(p) = self.active_provider_config() {
            if let Some(t) = p.max_tokens {
                return t;
            }
        }
        16384
    }

    /// Get the effective base URL.
    pub fn effective_base_url(&self) -> &str {
        if let Some(p) = self.active_provider_config() {
            if let Some(ref url) = p.base_url {
                return url;
            }
        }
        "https://api.anthropic.com"
    }

    /// Resolve the credential to use for API requests.
    /// Returns `(header_name, header_value)`.
    pub fn resolve_auth(&self) -> Option<(&'static str, String)> {
        let p = self.active_provider_config()?;
        if let Some(ref token) = p.auth_token {
            Some(("Authorization", format!("Bearer {}", token)))
        } else {
            p.api_key
                .as_ref()
                .map(|key| ("x-api-key", key.clone()))
        }
    }

    /// Get the API version header value (if configured).
    pub fn api_version(&self) -> Option<&str> {
        self.active_provider_config()?
            .api_version
            .as_deref()
    }

    /// Get the provider type (anthropic, openai, openai-compatible).
    pub fn provider_type(&self) -> &str {
        self.active_provider_config()
            .map(|p| p.provider_type.as_str())
            .unwrap_or("anthropic")
    }

    pub fn project_settings_path(project_root: &PathBuf) -> PathBuf {
        project_root.join(".venus").join("config.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let s = Settings::default();
        assert_eq!(s.effective_model(), "claude-sonnet-4-20250514");
        assert_eq!(s.effective_max_tokens(), 16384);
        assert_eq!(s.effective_base_url(), "https://api.anthropic.com");
    }

    #[test]
    fn test_provider_config() {
        let mut providers = HashMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig {
                provider_type: "anthropic".to_string(),
                api_key: Some("test-key".to_string()),
                auth_token: None,
                base_url: Some("https://custom.api.com".to_string()),
                default_model: Some("claude-opus-4-20250514".to_string()),
                max_tokens: Some(8192),
                api_version: None,
            },
        );

        let settings = Settings {
            active_provider: Some("anthropic".to_string()),
            provider: Some(providers),
            ..Default::default()
        };

        assert_eq!(settings.effective_model(), "claude-opus-4-20250514");
        assert_eq!(settings.effective_max_tokens(), 8192);
        assert_eq!(settings.effective_base_url(), "https://custom.api.com");

        let (header, value) = settings.resolve_auth().unwrap();
        assert_eq!(header, "x-api-key");
        assert_eq!(value, "test-key");
    }

    #[test]
    fn test_model_override() {
        let mut providers = HashMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig {
                provider_type: "anthropic".to_string(),
                api_key: None,
                auth_token: None,
                base_url: None,
                default_model: Some("claude-opus-4-20250514".to_string()),
                max_tokens: None,
                api_version: None,
            },
        );

        let settings = Settings {
            active_provider: Some("anthropic".to_string()),
            model: Some("claude-haiku-4-20250506".to_string()),
            provider: Some(providers),
            ..Default::default()
        };

        // model field overrides provider's default_model
        assert_eq!(settings.effective_model(), "claude-haiku-4-20250506");
    }

    #[test]
    fn test_toml_parse() {
        let toml_str = r#"
active_provider = "anthropic"

[provider.anthropic]
type = "anthropic"
api_key = "sk-test"
base_url = "https://api.anthropic.com"
default_model = "claude-sonnet-4-20250514"

[provider.custom]
type = "openai-compatible"
api_key = "custom-key"
base_url = "http://localhost:11434/v1"
default_model = "llama3"

[thinking]
mode = "adaptive"
"#;

        let settings: Settings = toml::from_str(toml_str).unwrap();
        assert_eq!(settings.active_provider, Some("anthropic".to_string()));
        assert!(settings.provider.is_some());
        let providers = settings.provider.as_ref().unwrap();
        assert!(providers.contains_key("anthropic"));
        assert!(providers.contains_key("custom"));
        assert_eq!(settings.thinking.as_ref().unwrap().mode, Some("adaptive".to_string()));
    }

    #[test]
    fn test_toml_with_hooks() {
        let toml_str = r#"
[provider.anthropic]
type = "anthropic"
api_key = "test"

[[hooks.PreToolUse]]
matcher = "Bash|Write"

[[hooks.PreToolUse.hooks]]
command = "echo check"
timeout = 5
once = true
"#;

        let settings: Settings = toml::from_str(toml_str).unwrap();
        let hooks = settings.hooks.as_ref().unwrap();
        let pre_tool = hooks.entries.get("PreToolUse").unwrap();
        assert_eq!(pre_tool.len(), 1);
        assert_eq!(pre_tool[0].matcher, Some("Bash|Write".to_string()));
        assert_eq!(pre_tool[0].hooks[0].command, "echo check");
        assert!(pre_tool[0].hooks[0].once);
    }

    #[test]
    fn test_merge_providers() {
        let mut providers1 = HashMap::new();
        providers1.insert(
            "anthropic".to_string(),
            ProviderConfig {
                provider_type: "anthropic".to_string(),
                api_key: Some("key1".to_string()),
                auth_token: None,
                base_url: None,
                default_model: None,
                max_tokens: None,
                api_version: None,
            },
        );

        let mut providers2 = HashMap::new();
        providers2.insert(
            "openai".to_string(),
            ProviderConfig {
                provider_type: "openai".to_string(),
                api_key: Some("key2".to_string()),
                auth_token: None,
                base_url: None,
                default_model: None,
                max_tokens: None,
                api_version: None,
            },
        );

        let mut s1 = Settings {
            provider: Some(providers1),
            ..Default::default()
        };
        let s2 = Settings {
            provider: Some(providers2),
            ..Default::default()
        };

        s1.merge(s2);
        let providers = s1.provider.as_ref().unwrap();
        assert!(providers.contains_key("anthropic"));
        assert!(providers.contains_key("openai"));
    }
}
