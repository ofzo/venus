use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct ListMcpResourcesTool;

#[async_trait]
impl Tool for ListMcpResourcesTool {
    fn name(&self) -> &str {
        "ListMcpResources"
    }

    fn description(&self) -> &str {
        "List available resources from MCP (Model Context Protocol) servers. Resources are read-only data sources exposed by MCP servers."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "server_name": {
                    "type": "string",
                    "description": "Optional MCP server name to filter resources. If omitted, lists resources from all configured servers."
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let server_name = input
            .get("server_name")
            .and_then(|v| v.as_str());

        // Check if any MCP servers are configured
        let mcp_servers = ctx
            .settings
            .mcp_servers
            .as_ref()
            .filter(|s| !s.is_empty());

        if let Some(servers) = mcp_servers {
            if let Some(filter) = server_name {
                if servers.contains_key(filter) {
                    Ok(ToolResult::text(format!(
                        "MCP server '{}' found but resource listing is not yet implemented. \
                         MCP resource support is coming in a future update.",
                        filter
                    )))
                } else {
                    Ok(ToolResult::error(format!(
                        "MCP server '{}' not found. Available servers: {}",
                        filter,
                        servers.keys().cloned().collect::<Vec<_>>().join(", ")
                    )))
                }
            } else {
                Ok(ToolResult::text(format!(
                    "Found {} configured MCP server(s): {}. \
                     Resource listing is not yet implemented — coming in a future update.",
                    servers.len(),
                    servers.keys().cloned().collect::<Vec<_>>().join(", ")
                )))
            }
        } else {
            Ok(ToolResult::text(
                "No MCP servers configured. Add MCP server configurations to your settings to use this tool.".to_string()
            ))
        }
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let server = input
            .get("server_name")
            .and_then(|v| v.as_str())
            .unwrap_or("all");
        format!("ListMcpResources: {}", server)
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
    async fn test_no_mcp_servers_configured() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = ListMcpResourcesTool;
        let input = serde_json::json!({});

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content[0].as_text().unwrap().contains("No MCP servers configured"));
    }

    #[tokio::test]
    async fn test_schema_validation() {
        let tool = ListMcpResourcesTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("server_name").is_some());
    }
}
