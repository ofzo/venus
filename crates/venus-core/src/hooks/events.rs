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
    PostToolUseFailure {
        session_id: String,
        tool_name: String,
        error: String,
    },
    SessionEnd {
        session_id: String,
        message_count: usize,
    },
    SubagentStart {
        session_id: String,
        description: String,
    },
    SubagentStop {
        session_id: String,
    },
    PermissionRequest {
        session_id: String,
        tool_name: String,
    },
    FileChanged {
        session_id: String,
        path: String,
        change_type: String,
    },
    TaskCreated {
        session_id: String,
        task_id: String,
        subject: String,
    },
    TaskCompleted {
        session_id: String,
        task_id: String,
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
            HookEvent::PostToolUseFailure { .. } => "PostToolUseFailure",
            HookEvent::SessionEnd { .. } => "SessionEnd",
            HookEvent::SubagentStart { .. } => "SubagentStart",
            HookEvent::SubagentStop { .. } => "SubagentStop",
            HookEvent::PermissionRequest { .. } => "PermissionRequest",
            HookEvent::FileChanged { .. } => "FileChanged",
            HookEvent::TaskCreated { .. } => "TaskCreated",
            HookEvent::TaskCompleted { .. } => "TaskCompleted",
        }
    }

    /// Returns the tool name for tool-related events, `None` for others.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            HookEvent::PreToolUse { tool_name, .. } => Some(tool_name),
            HookEvent::PostToolUse { tool_name, .. } => Some(tool_name),
            HookEvent::PostToolUseFailure { tool_name, .. } => Some(tool_name),
            HookEvent::PermissionRequest { tool_name, .. } => Some(tool_name),
            _ => None,
        }
    }

    /// Returns event-specific data as a JSON value.
    pub fn event_data(&self) -> Value {
        match self {
            HookEvent::PreToolUse { session_id, tool_name, tool_input } => serde_json::json!({
                "session_id": session_id,
                "tool_name": tool_name,
                "tool_input": tool_input,
            }),
            HookEvent::PostToolUse { session_id, tool_name, tool_input, tool_result, is_error } => serde_json::json!({
                "session_id": session_id,
                "tool_name": tool_name,
                "tool_input": tool_input,
                "tool_result": tool_result,
                "is_error": is_error,
            }),
            HookEvent::UserPromptSubmit { session_id, prompt } => serde_json::json!({
                "session_id": session_id,
                "prompt": prompt,
            }),
            HookEvent::SessionStart { session_id, cwd, model } => serde_json::json!({
                "session_id": session_id,
                "cwd": cwd,
                "model": model,
            }),
            HookEvent::PreCompact { session_id, message_count } => serde_json::json!({
                "session_id": session_id,
                "message_count": message_count,
            }),
            HookEvent::PostCompact { session_id, messages_before, messages_after } => serde_json::json!({
                "session_id": session_id,
                "messages_before": messages_before,
                "messages_after": messages_after,
            }),
            HookEvent::Stop { session_id } => serde_json::json!({
                "session_id": session_id,
            }),
            HookEvent::PostToolUseFailure { session_id, tool_name, error } => serde_json::json!({
                "session_id": session_id,
                "tool_name": tool_name,
                "error": error,
            }),
            HookEvent::SessionEnd { session_id, message_count } => serde_json::json!({
                "session_id": session_id,
                "message_count": message_count,
            }),
            HookEvent::SubagentStart { session_id, description } => serde_json::json!({
                "session_id": session_id,
                "description": description,
            }),
            HookEvent::SubagentStop { session_id } => serde_json::json!({
                "session_id": session_id,
            }),
            HookEvent::PermissionRequest { session_id, tool_name } => serde_json::json!({
                "session_id": session_id,
                "tool_name": tool_name,
            }),
            HookEvent::FileChanged { session_id, path, change_type } => serde_json::json!({
                "session_id": session_id,
                "path": path,
                "change_type": change_type,
            }),
            HookEvent::TaskCreated { session_id, task_id, subject } => serde_json::json!({
                "session_id": session_id,
                "task_id": task_id,
                "subject": subject,
            }),
            HookEvent::TaskCompleted { session_id, task_id } => serde_json::json!({
                "session_id": session_id,
                "task_id": task_id,
            }),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_name_all_variants() {
        let events = vec![
            (HookEvent::PreToolUse { session_id: "s".into(), tool_name: "t".into(), tool_input: Value::Null }, "PreToolUse"),
            (HookEvent::PostToolUse { session_id: "s".into(), tool_name: "t".into(), tool_input: Value::Null, tool_result: "r".into(), is_error: false }, "PostToolUse"),
            (HookEvent::UserPromptSubmit { session_id: "s".into(), prompt: "p".into() }, "UserPromptSubmit"),
            (HookEvent::SessionStart { session_id: "s".into(), cwd: "/".into(), model: "m".into() }, "SessionStart"),
            (HookEvent::PreCompact { session_id: "s".into(), message_count: 0 }, "PreCompact"),
            (HookEvent::PostCompact { session_id: "s".into(), messages_before: 0, messages_after: 0 }, "PostCompact"),
            (HookEvent::Stop { session_id: "s".into() }, "Stop"),
            (HookEvent::PostToolUseFailure { session_id: "s".into(), tool_name: "t".into(), error: "e".into() }, "PostToolUseFailure"),
            (HookEvent::SessionEnd { session_id: "s".into(), message_count: 5 }, "SessionEnd"),
            (HookEvent::SubagentStart { session_id: "s".into(), description: "d".into() }, "SubagentStart"),
            (HookEvent::SubagentStop { session_id: "s".into() }, "SubagentStop"),
            (HookEvent::PermissionRequest { session_id: "s".into(), tool_name: "t".into() }, "PermissionRequest"),
            (HookEvent::FileChanged { session_id: "s".into(), path: "f.rs".into(), change_type: "modified".into() }, "FileChanged"),
            (HookEvent::TaskCreated { session_id: "s".into(), task_id: "t1".into(), subject: "subj".into() }, "TaskCreated"),
            (HookEvent::TaskCompleted { session_id: "s".into(), task_id: "t1".into() }, "TaskCompleted"),
        ];

        for (event, expected_name) in events {
            assert_eq!(event.event_name(), expected_name);
        }
    }

    #[test]
    fn test_tool_name_returns_some_for_tool_events() {
        let tool_events: Vec<HookEvent> = vec![
            HookEvent::PreToolUse { session_id: "s".into(), tool_name: "Bash".into(), tool_input: Value::Null },
            HookEvent::PostToolUse { session_id: "s".into(), tool_name: "Edit".into(), tool_input: Value::Null, tool_result: "".into(), is_error: false },
            HookEvent::PostToolUseFailure { session_id: "s".into(), tool_name: "Write".into(), error: "fail".into() },
            HookEvent::PermissionRequest { session_id: "s".into(), tool_name: "Bash".into() },
        ];
        for event in &tool_events {
            assert!(event.tool_name().is_some(), "expected tool_name for {}", event.event_name());
        }
    }

    #[test]
    fn test_tool_name_returns_none_for_non_tool_events() {
        let non_tool_events: Vec<HookEvent> = vec![
            HookEvent::UserPromptSubmit { session_id: "s".into(), prompt: "p".into() },
            HookEvent::SessionStart { session_id: "s".into(), cwd: "/".into(), model: "m".into() },
            HookEvent::Stop { session_id: "s".into() },
            HookEvent::SessionEnd { session_id: "s".into(), message_count: 0 },
            HookEvent::SubagentStart { session_id: "s".into(), description: "d".into() },
            HookEvent::SubagentStop { session_id: "s".into() },
            HookEvent::FileChanged { session_id: "s".into(), path: "f".into(), change_type: "added".into() },
            HookEvent::TaskCreated { session_id: "s".into(), task_id: "t".into(), subject: "s".into() },
            HookEvent::TaskCompleted { session_id: "s".into(), task_id: "t".into() },
            HookEvent::PreCompact { session_id: "s".into(), message_count: 0 },
            HookEvent::PostCompact { session_id: "s".into(), messages_before: 0, messages_after: 0 },
        ];
        for event in &non_tool_events {
            assert!(event.tool_name().is_none(), "expected None tool_name for {}", event.event_name());
        }
    }

    #[test]
    fn test_event_data_serialization() {
        let event = HookEvent::PostToolUseFailure {
            session_id: "sess1".into(),
            tool_name: "Bash".into(),
            error: "command failed".into(),
        };
        let data = event.event_data();
        assert_eq!(data["session_id"], "sess1");
        assert_eq!(data["tool_name"], "Bash");
        assert_eq!(data["error"], "command failed");
    }

    #[test]
    fn test_event_data_session_end() {
        let event = HookEvent::SessionEnd {
            session_id: "sess1".into(),
            message_count: 42,
        };
        let data = event.event_data();
        assert_eq!(data["session_id"], "sess1");
        assert_eq!(data["message_count"], 42);
    }

    #[test]
    fn test_event_data_subagent_start() {
        let event = HookEvent::SubagentStart {
            session_id: "sess1".into(),
            description: "run tests".into(),
        };
        let data = event.event_data();
        assert_eq!(data["description"], "run tests");
    }

    #[test]
    fn test_event_data_file_changed() {
        let event = HookEvent::FileChanged {
            session_id: "sess1".into(),
            path: "src/main.rs".into(),
            change_type: "modified".into(),
        };
        let data = event.event_data();
        assert_eq!(data["path"], "src/main.rs");
        assert_eq!(data["change_type"], "modified");
    }

    #[test]
    fn test_event_data_task_created() {
        let event = HookEvent::TaskCreated {
            session_id: "sess1".into(),
            task_id: "task_1".into(),
            subject: "Fix bug".into(),
        };
        let data = event.event_data();
        assert_eq!(data["task_id"], "task_1");
        assert_eq!(data["subject"], "Fix bug");
    }

    #[test]
    fn test_event_data_task_completed() {
        let event = HookEvent::TaskCompleted {
            session_id: "sess1".into(),
            task_id: "task_1".into(),
        };
        let data = event.event_data();
        assert_eq!(data["task_id"], "task_1");
    }

    #[test]
    fn test_event_serialization_roundtrip() {
        let event = HookEvent::PermissionRequest {
            session_id: "sess1".into(),
            tool_name: "Bash".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("PermissionRequest"));
        assert!(json.contains("sess1"));
        assert!(json.contains("Bash"));
    }

    #[test]
    fn test_event_serialization_tagged() {
        let event = HookEvent::SubagentStop { session_id: "s".into() };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "SubagentStop");
        assert_eq!(json["session_id"], "s");
    }
}
