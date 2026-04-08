use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Events that can trigger hooks.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event")]
pub enum HookEvent {
    PreToolUse {
        session_id: String,
        tool_name: String,
        tool_input: Value,
    },
    PostToolUse {
        session_id: String,
        tool_name: String,
        tool_input: Value,
        tool_result: String,
        is_error: bool,
    },
    UserPromptSubmit {
        session_id: String,
        prompt: String,
    },
    SessionStart {
        session_id: String,
        cwd: String,
        model: String,
    },
    PreCompact {
        session_id: String,
        message_count: usize,
    },
    PostCompact {
        session_id: String,
        messages_before: usize,
        messages_after: usize,
    },
    Stop {
        session_id: String,
    },
}

impl HookEvent {
    /// Returns the event name string used to look up hooks in config.
    pub fn event_name(&self) -> &str {
        match self {
            HookEvent::PreToolUse { .. } => "PreToolUse",
            HookEvent::PostToolUse { .. } => "PostToolUse",
            HookEvent::UserPromptSubmit { .. } => "UserPromptSubmit",
            HookEvent::SessionStart { .. } => "SessionStart",
            HookEvent::PreCompact { .. } => "PreCompact",
            HookEvent::PostCompact { .. } => "PostCompact",
            HookEvent::Stop { .. } => "Stop",
        }
    }

    /// Returns the tool name for tool-related events, `None` for others.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            HookEvent::PreToolUse { tool_name, .. } => Some(tool_name),
            HookEvent::PostToolUse { tool_name, .. } => Some(tool_name),
            _ => None,
        }
    }
}

/// Response from a PreToolUse hook.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PreToolUseResponse {
    /// Decision: "allow", "deny", or "ask".
    pub decision: Option<String>,
    /// Reason for denial.
    pub reason: Option<String>,
    /// Modified tool input (replaces original if present).
    pub updated_input: Option<Value>,
}

/// Response from a UserPromptSubmit hook.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserPromptSubmitResponse {
    /// Modified prompt text (replaces original if present).
    pub updated_prompt: Option<String>,
    /// If true, block submission entirely.
    pub deny: Option<bool>,
    /// Reason for denial.
    pub reason: Option<String>,
}
