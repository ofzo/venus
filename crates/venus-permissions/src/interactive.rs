use async_trait::async_trait;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::io::Write;
use std::sync::{Arc, Mutex};
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
///
/// Session-level rules (from "always"/"never" choices) are stored with
/// interior mutability so they can be updated during permission prompts.
pub struct InteractivePermissionHandler {
    settings: Arc<Settings>,
    deny_rules: Vec<ParsedRule>,
    allow_rules: Vec<ParsedRule>,
    /// Rules added at runtime via "always allow" (a) in permission prompts.
    session_allow_rules: Mutex<Vec<ParsedRule>>,
    /// Rules added at runtime via "never" (d) in permission prompts.
    session_deny_rules: Mutex<Vec<ParsedRule>>,
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

        debug!(
            "permission handler: {} deny rules, {} allow rules",
            deny_rules.len(),
            allow_rules.len()
        );

        Self {
            settings,
            deny_rules,
            allow_rules,
            session_allow_rules: Mutex::new(Vec::new()),
            session_deny_rules: Mutex::new(Vec::new()),
        }
    }

    /// Add a session-level allow rule (from "always allow" prompt choice).
    fn add_session_allow_rule(&self, tool_name: &str) {
        let rule = ParsedRule {
            tool: tool_name.to_string(),
            pattern: None, // matches all inputs for this tool
        };
        if let Ok(mut rules) = self.session_allow_rules.lock() {
            debug!("adding session allow rule for tool: {}", tool_name);
            rules.push(rule);
        }
    }

    /// Add a session-level deny rule (from "never" prompt choice).
    fn add_session_deny_rule(&self, tool_name: &str) {
        let rule = ParsedRule {
            tool: tool_name.to_string(),
            pattern: None,
        };
        if let Ok(mut rules) = self.session_deny_rules.lock() {
            debug!("adding session deny rule for tool: {}", tool_name);
            rules.push(rule);
        }
    }

    /// Show the interactive permission prompt. Returns the user's decision.
    /// For "always"/"never" choices, persists the rule to session storage.
    fn prompt_user(&self, tool_name: &str, input: &Value) -> PermissionDecision {
        let description = format_tool_description(tool_name, input);
        let stderr = std::io::stderr();
        let mut out = stderr.lock();

        // Draw a styled permission box matching Claude Code's amber/yellow theme
        let _ = write!(out, "\r\n");
        let _ = write!(
            out,
            "  \x1b[33m⏺\x1b[0m \x1b[1;33m{}\x1b[0m\r\n",
            tool_name
        );
        let _ = write!(
            out,
            "  \x1b[2m{}\x1b[0m\r\n",
            description
        );
        let _ = write!(out, "\r\n");
        let _ = write!(
            out,
            "  \x1b[33my\x1b[0m allow  \
             \x1b[33mn\x1b[0m deny  \
             \x1b[33ma\x1b[0m always allow  \
             \x1b[33md\x1b[0m never\r\n"
        );
        let _ = write!(out, "  \x1b[2m>\x1b[0m ");
        let _ = out.flush();

        // Drop the lock before entering the blocking key-read loop
        drop(out);

        loop {
            if let Ok(Event::Key(KeyEvent { code, modifiers, .. })) = event::read() {
                let stderr = std::io::stderr();
                let mut out = stderr.lock();
                match code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        let _ = write!(out, "\r\n");
                        return PermissionDecision::Allow;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') => {
                        let _ = write!(out, "\r\n");
                        return PermissionDecision::Deny("user denied".to_string());
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        let _ = write!(out, "always\r\n");
                        drop(out);
                        self.add_session_allow_rule(tool_name);
                        return PermissionDecision::Allow;
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        let _ = write!(out, "never\r\n");
                        drop(out);
                        self.add_session_deny_rule(tool_name);
                        return PermissionDecision::Deny("user denied permanently".to_string());
                    }
                    KeyCode::Esc => {
                        let _ = write!(out, "\r\n");
                        return PermissionDecision::Deny("user cancelled".to_string());
                    }
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        let _ = write!(out, "\r\n");
                        return PermissionDecision::Deny("user cancelled".to_string());
                    }
                    _ => continue,
                }
            }
        }
    }
}

#[async_trait]
impl PermissionHandler for InteractivePermissionHandler {
    async fn check_permission(&self, tool_name: &str, input: &Value) -> PermissionDecision {
        // 1. Config deny rules — highest priority
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

        // 2. Session deny rules (from "never" choices)
        if let Ok(rules) = self.session_deny_rules.lock() {
            for rule in rules.iter() {
                if rule_matcher::rule_matches(rule, tool_name, input) {
                    debug!("permission denied by session rule for {}", tool_name);
                    return PermissionDecision::Deny("blocked by session rule".to_string());
                }
            }
        }

        // 3. Dangerous pattern check — always require approval
        if dangerous::is_dangerous(tool_name, input) {
            debug!("dangerous pattern detected for {}, prompting user", tool_name);
            return self.prompt_user(tool_name, input);
        }

        // 4. Read-only tools — auto-allow
        if READ_ONLY_TOOLS.contains(&tool_name) {
            return PermissionDecision::Allow;
        }

        // 5. Config allow rules
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

        // 6. Session allow rules (from "always allow" choices)
        if let Ok(rules) = self.session_allow_rules.lock() {
            for rule in rules.iter() {
                if rule_matcher::rule_matches(rule, tool_name, input) {
                    debug!("permission allowed by session rule for {}", tool_name);
                    return PermissionDecision::Allow;
                }
            }
        }

        // 7. Permission mode (read dynamically to support runtime mode cycling)
        let mode = self.settings
            .permission_mode
            .as_deref()
            .map(PermissionMode::parse_mode)
            .unwrap_or(PermissionMode::Default);
        match mode {
            PermissionMode::Bypass => {
                debug!("permission allowed by bypass mode");
                PermissionDecision::Allow
            }
            PermissionMode::Default | PermissionMode::Plan | PermissionMode::Auto => {
                self.prompt_user(tool_name, input)
            }
        }
    }
}

fn format_tool_description(tool_name: &str, input: &Value) -> String {
    match tool_name {
        "Bash" | "BashTool" => {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("command: {}", cmd)
        }
        "Write" | "FileWriteTool" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("write to: {}", path)
        }
        "Edit" | "FileEditTool" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("edit: {}", path)
        }
        "WebFetch" | "WebFetchTool" => {
            let url = input
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("fetch: {}", url)
        }
        "WebSearch" | "WebSearchTool" => {
            let query = input
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("search: {}", query)
        }
        "Agent" | "AgentTool" => {
            let desc = input
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("agent: {}", desc)
        }
        _ => serde_json::to_string(input).unwrap_or_default(),
    }
}
