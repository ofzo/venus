use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct SyntheticOutputTool;

#[async_trait]
impl Tool for SyntheticOutputTool {
    fn name(&self) -> &str {
        "SyntheticOutput"
    }

    fn description(&self) -> &str {
        "Return structured JSON output. Useful for returning structured data from tool calls."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "output": {
                    "description": "Any JSON value to return as formatted text output"
                }
            },
            "required": ["output"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let output = input
            .get("output")
            .ok_or_else(|| anyhow::anyhow!("missing 'output' parameter"))?;

        let formatted = match output {
            Value::String(s) => s.clone(),
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            _ => serde_json::to_string_pretty(output).unwrap_or_else(|_| output.to_string()),
        };

        Ok(ToolResult::text(formatted))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let output = input.get("output");
        match output {
            Some(Value::String(s)) => {
                let preview = if s.len() > 60 { format!("{}...", &s[..60]) } else { s.clone() };
                format!("SyntheticOutput: \"{}\"", preview)
            }
            Some(v) => {
                let s = serde_json::to_string(v).unwrap_or_default();
                let preview = if s.len() > 60 { format!("{}...", &s[..60]) } else { s };
                format!("SyntheticOutput: {}", preview)
            }
            None => "SyntheticOutput: (null)".to_string(),
        }
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
    async fn test_string_output() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = SyntheticOutputTool;
        let input = serde_json::json!({ "output": "hello world" });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content[0].as_text().unwrap(), "hello world");
    }

    #[tokio::test]
    async fn test_object_output() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = SyntheticOutputTool;
        let input = serde_json::json!({ "output": {"key": "value", "num": 42} });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("key"));
        assert!(text.contains("value"));
    }

    #[tokio::test]
    async fn test_various_json_types() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = SyntheticOutputTool;

        // Null
        let result = tool.execute(serde_json::json!({"output": null}), &ctx).await.unwrap();
        assert_eq!(result.content[0].as_text().unwrap(), "null");

        // Bool
        let result = tool.execute(serde_json::json!({"output": true}), &ctx).await.unwrap();
        assert_eq!(result.content[0].as_text().unwrap(), "true");

        // Number
        let result = tool.execute(serde_json::json!({"output": 3.14}), &ctx).await.unwrap();
        assert_eq!(result.content[0].as_text().unwrap(), "3.14");
    }
}
