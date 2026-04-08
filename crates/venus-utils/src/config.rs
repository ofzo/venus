use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::debug;

use crate::fs_helpers;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub custom_system_prompt: Option<String>,
    #[serde(default)]
    pub always_allow: Option<Vec<PermissionRule>>,
    #[serde(default)]
    pub always_deny: Option<Vec<PermissionRule>>,
    #[serde(default)]
    pub hooks: Option<HookConfig>,
    #[serde(default)]
    pub thinking: Option<ThinkingConfig>,
}

/// Extended thinking configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct HookEntry {
    /// Tool name pattern for tool events (pipe-separated: "Edit|Write|Bash").
    /// Ignored for non-tool events.
    #[serde(default)]
    pub matcher: Option<String>,
    /// The hook definitions to execute.
    #[serde(default)]
    pub hooks: Vec<CommandHook>,
}

/// A shell command to execute as a hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandHook {
    /// Hook type (currently only "command").
    #[serde(default, rename = "type")]
    pub hook_type: Option<String>,
    /// Shell command string (executed via `sh -c`).
    pub command: String,
    /// Timeout in seconds (default: 10).
    #[serde(default = "default_hook_timeout")]
    pub timeout: u64,
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

impl Settings {
    pub fn load() -> Result<Self> {
        let mut settings = Settings::default();

        // Load from user settings file
        if let Ok(config_dir) = fs_helpers::claude_config_dir() {
            let settings_path = config_dir.join("settings.json");
            if settings_path.exists() {
                let content = std::fs::read_to_string(&settings_path)?;
                let file_settings: Settings = serde_json::from_str(&content)?;
                settings.merge(file_settings);
                debug!("loaded settings from {:?}", settings_path);
            }
        }

        // Override with environment variables
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            settings.api_key = Some(key);
        }
        if let Ok(url) = std::env::var("ANTHROPIC_BASE_URL") {
            settings.base_url = Some(url);
        }
        if let Ok(model) = std::env::var("ANTHROPIC_MODEL") {
            settings.model = Some(model);
        }

        Ok(settings)
    }

    fn merge(&mut self, other: Settings) {
        if other.model.is_some() {
            self.model = other.model;
        }
        if other.max_tokens.is_some() {
            self.max_tokens = other.max_tokens;
        }
        if other.api_key.is_some() {
            self.api_key = other.api_key;
        }
        if other.base_url.is_some() {
            self.base_url = other.base_url;
        }
        if other.permission_mode.is_some() {
            self.permission_mode = other.permission_mode;
        }
        if other.custom_system_prompt.is_some() {
            self.custom_system_prompt = other.custom_system_prompt;
        }
        if other.always_allow.is_some() {
            self.always_allow = other.always_allow;
        }
        if other.always_deny.is_some() {
            self.always_deny = other.always_deny;
        }
        if other.hooks.is_some() {
            self.hooks = other.hooks;
        }
        if other.thinking.is_some() {
            self.thinking = other.thinking;
        }
    }

    pub fn effective_model(&self) -> &str {
        self.model.as_deref().unwrap_or("claude-sonnet-4-20250514")
    }

    pub fn effective_max_tokens(&self) -> u32 {
        self.max_tokens.unwrap_or(16384)
    }

    pub fn effective_base_url(&self) -> &str {
        self.base_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com")
    }

    pub fn project_settings_path(project_root: &PathBuf) -> PathBuf {
        project_root.join(".claude").join("settings.json")
    }
}
