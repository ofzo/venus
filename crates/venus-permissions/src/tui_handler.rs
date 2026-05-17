use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use tracing::debug;
use venus_core::tool::{PermissionDecision, PermissionHandler};
use venus_utils::config::Settings;

use crate::dangerous;
use crate::rule_matcher;
use crate::rule_parser::{self, ParsedRule};
use crate::types::PermissionMode;

/// Read-only tools that are always auto-allowed.
const READ_ONLY_TOOLS: &[&str] = &["Read", "Glob", "Grep"];

/// A permission request sent from the engine to the TUI.
pub struct PermissionRequest {
    pub tool_name: String,
    pub description: String,
    pub response_tx: oneshot::Sender<PermissionResponse>,
}

/// The user's response to a permission request.
pub enum PermissionResponse {
    Allow,
    Deny,
    AlwaysAllow,
    NeverAllow,
}

/// Channel-based permission handler for TUI mode.
///
/// Instead of blocking on terminal I/O, this handler sends permission requests
/// through a channel to the TUI event loop and awaits the response.
pub struct TuiPermissionHandler {
    settings: Arc<Settings>,
    deny_rules: Vec<ParsedRule>,
    allow_rules: Vec<ParsedRule>,
    session_allow_rules: Mutex<Vec<ParsedRule>>,
    session_deny_rules: Mutex<Vec<ParsedRule>>,
    request_tx: mpsc::UnboundedSender<PermissionRequest>,
}

impl TuiPermissionHandler {
    pub fn new(
        settings: Arc<Settings>,
        request_tx: mpsc::UnboundedSender<PermissionRequest>,
    ) -> Self {
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
            "TUI permission handler: {} deny rules, {} allow rules",
            deny_rules.len(),
            allow_rules.len()
        );

        Self {
            settings,
            deny_rules,
            allow_rules,
            session_allow_rules: Mutex::new(Vec::new()),
            session_deny_rules: Mutex::new(Vec::new()),
            request_tx,
        }
    }

    fn add_session_allow_rule(&self, tool_name: &str) {
        let rule = ParsedRule {
            tool: tool_name.to_string(),
            pattern: None,
        };
        if let Ok(mut rules) = self.session_allow_rules.lock() {
            debug!("adding session allow rule for tool: {}", tool_name);
            rules.push(rule);
        }
    }

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

    /// Send a permission request to the TUI and await the response.
    async fn prompt_user(&self, tool_name: &str, input: &Value) -> PermissionDecision {
        let description = format_tool_description(tool_name, input);
        let (response_tx, response_rx) = oneshot::channel();

        let request = PermissionRequest {
            tool_name: tool_name.to_string(),
            description,
            response_tx,
        };

        if self.request_tx.send(request).is_err() {
            debug!("permission request channel closed, denying");
            return PermissionDecision::Deny("channel closed".to_string());
        }

        // Wait for the TUI to send back the user's decision
        match response_rx.await {
            Ok(PermissionResponse::Allow) => PermissionDecision::Allow,
            Ok(PermissionResponse::Deny) => {
                PermissionDecision::Deny("user denied".to_string())
            }
            Ok(PermissionResponse::AlwaysAllow) => {
                self.add_session_allow_rule(tool_name);
                PermissionDecision::Allow
            }
            Ok(PermissionResponse::NeverAllow) => {
                self.add_session_deny_rule(tool_name);
                PermissionDecision::Deny("user denied permanently".to_string())
            }
            Err(_) => PermissionDecision::Deny("channel closed".to_string()),
        }
    }
}

#[async_trait]
impl PermissionHandler for TuiPermissionHandler {
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

        // 2. Session deny rules
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
            return self.prompt_user(tool_name, input).await;
        }

        // 4. Read-only tools — auto-allow
        if READ_ONLY_TOOLS.iter().any(|&t| t == tool_name) {
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

        // 6. Session allow rules
        if let Ok(rules) = self.session_allow_rules.lock() {
            for rule in rules.iter() {
                if rule_matcher::rule_matches(rule, tool_name, input) {
                    debug!("permission allowed by session rule for {}", tool_name);
                    return PermissionDecision::Allow;
                }
            }
        }

        // 7. Permission mode
        let mode = self
            .settings
            .permission_mode
            .as_deref()
            .map(PermissionMode::from_str)
            .unwrap_or(PermissionMode::Default);
        match mode {
            PermissionMode::Bypass => {
                debug!("permission allowed by bypass mode");
                PermissionDecision::Allow
            }
            PermissionMode::Default | PermissionMode::Plan | PermissionMode::Auto => {
                self.prompt_user(tool_name, input).await
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
