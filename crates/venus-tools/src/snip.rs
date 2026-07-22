use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct SnipTool;

#[async_trait]
impl Tool for SnipTool {
    fn name(&self) -> &str {
        "Snip"
    }

    fn description(&self) -> &str {
        "Trim conversation history by removing older messages. Returns an instruction for the system to apply the trim."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "keep_last": {
                    "type": "number",
                    "description": "Number of recent messages to keep (default: 10)"
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let keep_last = input
            .get("keep_last")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        if keep_last == 0 {
            return Ok(ToolResult::error(
                "keep_last must be at least 1. Cannot trim all messages.".to_string(),
            ));
        }

        if keep_last > 1000 {
            return Ok(ToolResult::error(
                "keep_last cannot exceed 1000. Please use a smaller value.".to_string(),
            ));
        }

        let mut messages = ctx.messages.lock().await;
        let total = messages.len();

        if total <= keep_last {
            return Ok(ToolResult::text(format!(
                "No snip needed: conversation has {} messages, keeping last {}.",
                total, keep_last
            )));
        }

        let removed = total - keep_last;
        messages.drain(..removed);

        Ok(ToolResult::text(format!(
            "Snipped {} messages. Conversation now has {} messages (kept last {}).",
            removed, messages.len(), keep_last
        )))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let keep = input.get("keep_last").and_then(|v| v.as_u64()).unwrap_or(10);
        format!("Snip: keep last {} messages", keep)
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
            cost_tracker: None,
        }
    }

    #[tokio::test]
    async fn test_default_keep_last() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = SnipTool;
        let input = serde_json::json!({});

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content[0].as_text().unwrap().contains("No snip needed"));
    }

    #[tokio::test]
    async fn test_keep_last_zero_invalid() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = SnipTool;
        let input = serde_json::json!({ "keep_last": 0 });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_keep_last_exceeds_max() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = SnipTool;
        let input = serde_json::json!({ "keep_last": 5000 });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
    }
}
