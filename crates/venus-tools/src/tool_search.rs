use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct ToolSearchTool;

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "ToolSearch"
    }

    fn description(&self) -> &str {
        "Search available tools by keyword. Returns matching tool names and descriptions."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query to match against tool names and descriptions"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;

        let query_lower = query.to_lowercase();

        let matches: Vec<Value> = ctx
            .tools
            .all()
            .iter()
            .filter(|tool| {
                let name_match = tool.name().to_lowercase().contains(&query_lower);
                let desc_match = tool.description().to_lowercase().contains(&query_lower);
                name_match || desc_match
            })
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                })
            })
            .collect();

        if matches.is_empty() {
            Ok(ToolResult::text(format!(
                "No tools found matching '{}'. Available tools: {}",
                query,
                ctx.tools
                    .all()
                    .iter()
                    .map(|t| t.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        } else {
            let response = serde_json::json!({
                "query": query,
                "count": matches.len(),
                "tools": matches,
            });
            Ok(ToolResult::text(
                serde_json::to_string_pretty(&response).unwrap_or_default(),
            ))
        }
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let query = input.get("query").and_then(|v| v.as_str()).unwrap_or("?");
        format!("ToolSearch: {}", query)
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

    fn make_context(dir: &std::path::Path, tools: Vec<Box<dyn venus_core::tool::Tool>>) -> ToolContext {
        ToolContext {
            working_dir: dir.to_path_buf(),
            session_id: "test-session".to_string(),
            cancel_token: CancellationToken::new(),
            permission_handler: Arc::new(NoopPermission),
            settings: Arc::new(Settings::default()),
            task_store: Arc::new(TaskStore::new()),
            background_runtime: Arc::new(BackgroundTaskRuntime::new()),
            plan_mode: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            auth_header: "",
            auth_value: String::new(),
            base_url: String::new(),
            model: String::new(),
            tools: Arc::new(ToolRegistry::new(tools)),
            hook_runner: Arc::new(HookRunner::new(None, "test-session".to_string(), dir.to_path_buf())),
            cron_scheduler: None,
        }
    }

    #[tokio::test]
    async fn test_exact_match() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(
            tmp.path(),
            vec![
                Box::new(crate::bash::BashTool),
                Box::new(crate::web_search::WebSearchTool),
            ],
        );

        let tool = ToolSearchTool;
        let input = serde_json::json!({ "query": "Bash" });
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content[0].as_text().unwrap().contains("Bash"));
    }

    #[tokio::test]
    async fn test_partial_match() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(
            tmp.path(),
            vec![
                Box::new(crate::bash::BashTool),
                Box::new(crate::web_search::WebSearchTool),
            ],
        );

        let tool = ToolSearchTool;
        let input = serde_json::json!({ "query": "search" });
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content[0].as_text().unwrap().contains("WebSearch"));
    }

    #[tokio::test]
    async fn test_no_match() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(
            tmp.path(),
            vec![Box::new(crate::bash::BashTool)],
        );

        let tool = ToolSearchTool;
        let input = serde_json::json!({ "query": "nonexistent_xyz" });
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content[0].as_text().unwrap().contains("No tools found"));
    }
}
