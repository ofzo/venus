use async_trait::async_trait;
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use serde_json::Value;
use std::io::Write;
use std::sync::Arc;
use tracing::debug;
use venus_core::tool::{PermissionDecision, PermissionHandler};
use venus_utils::config::Settings;

use crate::dangerous;
use crate::rule_matcher;
use crate::rule_parser::{self, ParsedRule};
use crate::types::PermissionMode;

/// Read-only tools that are always auto-allowed.
const READ_ONLY_TOOLS: &[&str] = &["Read", "Glob", "Grep"];

/// Interactive permission handler that evaluates deny/allow rules,
/// checks permission mode, and falls back to terminal prompting.
pub struct InteractivePermissionHandler {
    #[allow(dead_code)]
    settings: Arc<Settings>,
    deny_rules: Vec<ParsedRule>,
    allow_rules: Vec<ParsedRule>,
    mode: PermissionMode,
}

impl InteractivePermissionHandler {
    pub fn new(settings: Arc<Settings>) -> Self {
        let deny_rules: Vec<ParsedRule> = settings
            .always_deny
            .as_ref()
            .map(|rules| rules.iter().map(rule_parser::parse_rule).collect())
            .unwrap_or_default();
        let allow_rules: Vec<ParsedRule> = settings
            .always_allow
            .as_ref()
            .map(|rules| rules.iter().map(rule_parser::parse_rule).collect())
            .unwrap_or_default();
        let mode = settings
            .permission_mode
            .as_deref()
            .map(PermissionMode::from_str)
            .unwrap_or(PermissionMode::Default);

        debug!(
            "permission handler: mode={:?}, {} deny rules, {} allow rules",
            mode,
            deny_rules.len(),
            allow_rules.len()
        );

        Self {
            settings,
            deny_rules,
            allow_rules,
            mode,
        }
    }
}

#[async_trait]
impl PermissionHandler for InteractivePermissionHandler {
    async fn check_permission(&self, tool_name: &str, input: &Value) -> PermissionDecision {
        // 1. Deny rules — highest priority
        for rule in &self.deny_rules {
            if rule_matcher::rule_matches(rule, tool_name, input) {
                let desc = format!(
                    "blocked by deny rule: {}({})",
                    rule.tool,
                    rule.pattern.as_deref().unwrap_or("*")
                );
                debug!("permission denied: {}", desc);
                return PermissionDecision::Deny(desc);
            }
        }

        // 2. Dangerous pattern check — always require approval
        if dangerous::is_dangerous(tool_name, input) {
            debug!("dangerous pattern detected for {}, prompting user", tool_name);
            return prompt_user(tool_name, input);
        }

        // 3. Read-only tools — auto-allow
        if READ_ONLY_TOOLS.iter().any(|&t| t == tool_name) {
            return PermissionDecision::Allow;
        }

        // 4. Allow rules
        for rule in &self.allow_rules {
            if rule_matcher::rule_matches(rule, tool_name, input) {
                debug!(
                    "permission allowed by rule: {}({})",
                    rule.tool,
                    rule.pattern.as_deref().unwrap_or("*")
                );
                return PermissionDecision::Allow;
            }
        }

        // 5. Permission mode
        match self.mode {
            PermissionMode::Bypass => {
                debug!("permission allowed by bypass mode");
                PermissionDecision::Allow
            }
            PermissionMode::Default | PermissionMode::Plan | PermissionMode::Auto => {
                prompt_user(tool_name, input)
            }
        }
    }
}

/// Interactive y/n prompt via crossterm key events.
fn prompt_user(tool_name: &str, input: &Value) -> PermissionDecision {
    let description = format_tool_description(tool_name, input);
    eprint!(
        "\n  Tool: {}\n  {}\n  Allow? (y/n): ",
        tool_name, description
    );
    std::io::stderr().flush().ok();

    loop {
        if let Ok(Event::Key(KeyEvent { code, .. })) = event::read() {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    eprintln!("y");
                    return PermissionDecision::Allow;
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    eprintln!("n");
                    return PermissionDecision::Deny("user denied".to_string());
                }
                _ => continue,
            }
        }
    }
}

fn format_tool_description(tool_name: &str, input: &Value) -> String {
    match tool_name {
        "Bash" => {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("command: {}", cmd)
        }
        "Write" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("write to: {}", path)
        }
        "Edit" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("edit: {}", path)
        }
        _ => serde_json::to_string(input).unwrap_or_default(),
    }
}
