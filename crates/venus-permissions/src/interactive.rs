use async_trait::async_trait;
use venus_core::tool::{PermissionDecision, PermissionHandler};
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use serde_json::Value;
use std::io::Write;
use std::sync::Mutex;

/// Read-only tools that are always allowed without prompting.
const AUTO_ALLOW_TOOLS: &[&str] = &["Read", "Glob", "Grep"];

/// Interactive permission handler that prompts the user in the terminal.
pub struct InteractivePermissionHandler {
    /// Mutex to serialize permission prompts (only one at a time).
    _lock: Mutex<()>,
}

impl InteractivePermissionHandler {
    pub fn new() -> Self {
        Self {
            _lock: Mutex::new(()),
        }
    }
}

impl Default for InteractivePermissionHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PermissionHandler for InteractivePermissionHandler {
    async fn check_permission(
        &self,
        tool_name: &str,
        input: &Value,
    ) -> PermissionDecision {
        // Auto-allow read-only tools
        if AUTO_ALLOW_TOOLS.iter().any(|&t| t == tool_name) {
            return PermissionDecision::Allow;
        }

        // For other tools, prompt in terminal
        let description = format_tool_description(tool_name, input);
        eprint!("\n  Tool: {}\n  {}\n  Allow? (y/n): ", tool_name, description);
        std::io::stderr().flush().ok();

        // Read single keypress
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
}

fn format_tool_description(tool_name: &str, input: &Value) -> String {
    match tool_name {
        "Bash" => {
            let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            format!("command: {}", cmd)
        }
        "Write" => {
            let path = input.get("file_path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("write to: {}", path)
        }
        "Edit" => {
            let path = input.get("file_path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("edit: {}", path)
        }
        _ => serde_json::to_string(input).unwrap_or_default(),
    }
}
