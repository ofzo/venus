use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use venus_core::plugin::PluginToolDef;
use venus_core::tool::{Tool, ToolContext, ToolResult};
use serde_json::Value;

/// Wraps a plugin's external command as a [`Tool`] implementation.
///
/// When executed, the input JSON is piped to the command's stdin and the
/// command's stdout is returned as the tool result.
pub struct PluginTool {
    pub tool_def: PluginToolDef,
    pub base_dir: PathBuf,
}

#[async_trait]
impl Tool for PluginTool {
    fn name(&self) -> &str {
        &self.tool_def.name
    }

    fn description(&self) -> &str {
        &self.tool_def.description
    }

    fn input_schema(&self) -> Value {
        self.tool_def
            .input_schema
            .clone()
            .unwrap_or_else(|| {
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                })
            })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let input_json = serde_json::to_string(&input)
            .context("failed to serialize tool input")?;

        let mut child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(&self.tool_def.command)
            .current_dir(&self.base_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn plugin command: {}", self.tool_def.command))?;

        // Write JSON input to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input_json.as_bytes())
                .await
                .context("failed to write to plugin stdin")?;
            stdin.shutdown().await.ok();
        }

        let timeout = std::time::Duration::from_secs(30);
        let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if output.status.success() {
                    Ok(ToolResult::text(stdout))
                } else {
                    let mut msg = stdout;
                    if !stderr.is_empty() {
                        if !msg.is_empty() {
                            msg.push('\n');
                        }
                        msg.push_str(&stderr);
                    }
                    let code = output.status.code().unwrap_or(-1);
                    Ok(ToolResult::error(format!(
                        "{}\nexit code: {}",
                        msg, code
                    )))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!(
                "failed to run plugin command: {}",
                e
            ))),
            Err(_elapsed) => {
                // Timeout - child was consumed by wait_with_output, process will be cleaned up
                Ok(ToolResult::error("plugin command timed out after 30s"))
            }
        }
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn format_for_display(&self, input: &Value) -> String {
        format!(
            "plugin:{} {}",
            self.tool_def.name,
            serde_json::to_string(input).unwrap_or_default()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_tool(command: &str) -> PluginTool {
        PluginTool {
            tool_def: PluginToolDef {
                name: "echo_tool".to_string(),
                description: "Echoes input".to_string(),
                command: command.to_string(),
                input_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" }
                    }
                })),
            },
            base_dir: PathBuf::from("/tmp"),
        }
    }

    #[test]
    fn test_plugin_tool_name_and_description() {
        let tool = make_tool("echo hello");
        assert_eq!(tool.name(), "echo_tool");
        assert_eq!(tool.description(), "Echoes input");
    }

    #[test]
    fn test_plugin_tool_input_schema() {
        let tool = make_tool("echo hello");
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["text"].is_object());
    }

    #[test]
    fn test_plugin_tool_input_schema_default() {
        let tool = PluginTool {
            tool_def: PluginToolDef {
                name: "no_schema".to_string(),
                description: "No schema".to_string(),
                command: "echo ok".to_string(),
                input_schema: None,
            },
            base_dir: PathBuf::from("/tmp"),
        };
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
    }

    #[test]
    fn test_plugin_tool_format_for_display() {
        let tool = make_tool("echo hello");
        let input = serde_json::json!({"text": "world"});
        let display = tool.format_for_display(&input);
        assert!(display.contains("echo_tool"));
        assert!(display.contains("world"));
    }

    #[test]
    fn test_plugin_tool_is_not_read_only() {
        let tool = make_tool("echo hello");
        assert!(!tool.is_read_only());
    }

    #[tokio::test]
    async fn test_plugin_tool_execute_success() {
        let tool = make_tool("cat");
        // cat reads stdin and outputs it
        let input = serde_json::json!({"text": "hello world"});
        let ctx = make_test_context();
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = match result.content.first().unwrap() {
            venus_core::message::ContentBlock::Text { text } => text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("hello world"));
    }

    #[tokio::test]
    async fn test_plugin_tool_execute_failure() {
        let tool = make_tool("exit 1");
        let input = serde_json::json!({});
        let ctx = make_test_context();
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_plugin_tool_execute_with_working_dir() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("data.txt"), "from file").unwrap();

        let tool = PluginTool {
            tool_def: PluginToolDef {
                name: "reader".to_string(),
                description: "Reads a file".to_string(),
                command: "cat data.txt".to_string(),
                input_schema: None,
            },
            base_dir: tmp.path().to_path_buf(),
        };

        let ctx = make_test_context();
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = match result.content.first().unwrap() {
            venus_core::message::ContentBlock::Text { text } => text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("from file"));
    }

    fn make_test_context() -> ToolContext {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        use venus_core::background::BackgroundTaskRuntime;
        use venus_core::task::TaskStore;
        use venus_utils::config::Settings;

        struct NoopPerm;
        #[async_trait]
        impl venus_core::tool::PermissionHandler for NoopPerm {
            async fn check_permission(
                &self,
                _tool_name: &str,
                _input: &Value,
            ) -> venus_core::tool::PermissionDecision {
                venus_core::tool::PermissionDecision::Allow
            }
        }

        ToolContext {
            working_dir: PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            cancel_token: tokio_util::sync::CancellationToken::new(),
            permission_handler: Arc::new(NoopPerm),
            settings: Arc::new(Settings::default()),
            task_store: Arc::new(TaskStore::new()),
            background_runtime: Arc::new(BackgroundTaskRuntime::new()),
            plan_mode: Arc::new(AtomicBool::new(false)),
            messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            auth_header: "",
            auth_value: String::new(),
            base_url: String::new(),
            model: String::new(),
            tools: Arc::new(venus_core::tool_registry::ToolRegistry::new(vec![])),
            hook_runner: Arc::new(venus_core::hooks::HookRunner::new(None, String::new(), PathBuf::from("/tmp"))),
            cron_scheduler: None,
        }
    }
}
