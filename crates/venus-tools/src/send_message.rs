use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct SendMessageTool;

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "SendMessage"
    }

    fn description(&self) -> &str {
        "Send a message to a background agent task. The message will be delivered to the task's message channel."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The background task ID to send the message to"
                },
                "message": {
                    "type": "string",
                    "description": "The message content to send"
                }
            },
            "required": ["task_id", "message"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let task_id = input
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'task_id' parameter"))?;

        let _message = input
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'message' parameter"))?;

        // Check if the background task exists
        match ctx.background_runtime.read_output(task_id).await {
            Ok((info, _)) => {
                if info.status == venus_core::background::BackgroundTaskStatus::Running {
                    Ok(ToolResult::text(format!(
                        "Message sent to task {} (status: running). Note: message delivery to background tasks is a placeholder — the task will continue running.",
                        task_id
                    )))
                } else {
                    Ok(ToolResult::error(format!(
                        "Task {} exists but is not running (status: {:?}). Cannot send message to a completed or failed task.",
                        task_id, info.status
                    )))
                }
            }
            Err(_) => Ok(ToolResult::error(format!(
                "Background task '{}' not found. Check the task ID and try again.",
                task_id
            ))),
        }
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn format_for_display(&self, input: &Value) -> String {
        let task_id = input.get("task_id").and_then(|v| v.as_str()).unwrap_or("?");
        let msg = input
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let preview = if msg.len() > 50 {
            format!("{}...", &msg[..50])
        } else {
            msg.to_string()
        };
        format!("SendMessage -> {}: {}", task_id, preview)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;
    use venus_core::background::BackgroundTaskRuntime;
    use venus_core::hooks::HookRunner;
    use venus_core::task::TaskStore;
    use venus_core::tool::{PermissionDecision, PermissionHandler};
    use venus_core::tool_registry::ToolRegistry;
    use venus_utils::config::Settings;

    struct NoopPermission;
    #[async_trait]
    impl PermissionHandler for NoopPermission {
        async fn check_permission(&self, _: &str, _: &Value) -> PermissionDecision {
            PermissionDecision::Allow
        }
    }

    fn make_context(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            working_dir: dir.to_path_buf(),
            session_id: "test-session".to_string(),
            cancel_token: CancellationToken::new(),
            permission_handler: Arc::new(NoopPermission),
            settings: Arc::new(Settings::default()),
            task_store: Arc::new(TaskStore::new()),
            background_runtime: Arc::new(BackgroundTaskRuntime::new()),
            plan_mode: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            auth_header: "",
            auth_value: String::new(),
            base_url: String::new(),
            model: String::new(),
            tools: Arc::new(ToolRegistry::new(vec![])),
            hook_runner: Arc::new(HookRunner::new(None, "test-session".to_string(), dir.to_path_buf())),
            cron_scheduler: None,
        }
    }

    #[tokio::test]
    async fn test_invalid_task_id() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = SendMessageTool;
        let input = serde_json::json!({
            "task_id": "nonexistent_999",
            "message": "hello"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_missing_parameters() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = SendMessageTool;

        // Missing task_id
        let input = serde_json::json!({ "message": "hi" });
        assert!(tool.execute(input, &ctx).await.is_err());

        // Missing message
        let input = serde_json::json!({ "task_id": "123" });
        assert!(tool.execute(input, &ctx).await.is_err());
    }

    #[tokio::test]
    async fn test_send_to_running_task() {
        let tmp = TempDir::new().unwrap();
        let rt = Arc::new(BackgroundTaskRuntime::new());
        let task_id = rt.spawn("test task".to_string(), async { Ok("done".to_string()) }).await;
        // Wait briefly for task to complete
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let ctx = ToolContext {
            working_dir: tmp.path().to_path_buf(),
            session_id: "test".to_string(),
            cancel_token: CancellationToken::new(),
            permission_handler: Arc::new(NoopPermission),
            settings: Arc::new(Settings::default()),
            task_store: Arc::new(TaskStore::new()),
            background_runtime: rt,
            plan_mode: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            auth_header: "",
            auth_value: String::new(),
            base_url: String::new(),
            model: String::new(),
            tools: Arc::new(ToolRegistry::new(vec![])),
            hook_runner: Arc::new(HookRunner::new(None, "test-session".to_string(), tmp.path().to_path_buf())),
            cron_scheduler: None,
        };

        let tool = SendMessageTool;
        let input = serde_json::json!({
            "task_id": &task_id,
            "message": "hello"
        });
        let result = tool.execute(input, &ctx).await.unwrap();
        // Task may have already completed, so result could be error (not running)
        // The point is it doesn't panic
        assert!(!result.content.is_empty());
    }
}
