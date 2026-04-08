use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use venus_core::tool::{Tool, ToolContext, ToolResult};

use crate::client::McpClient;
use crate::client::McpToolDef;

/// Wraps an MCP server tool as a venus-core Tool.
pub struct McpTool {
    /// Composite name: mcp__{server_name}__{tool_name}
    qualified_name: String,
    tool_def: McpToolDef,
    client: Arc<McpClient>,
}

impl McpTool {
    pub fn new(server_name: &str, tool_def: McpToolDef, client: Arc<McpClient>) -> Self {
        let qualified_name = format!("mcp__{}__{}", server_name, tool_def.name);
        Self {
            qualified_name,
            tool_def,
            client,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.qualified_name
    }

    fn description(&self) -> &str {
        &self.tool_def.description
    }

    fn input_schema(&self) -> Value {
        self.tool_def.input_schema.clone()
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        match self.client.call_tool(&self.tool_def.name, input).await {
            Ok(value) => {
                let text = match value {
                    Value::String(s) => s,
                    other => serde_json::to_string_pretty(&other).unwrap_or_default(),
                };
                Ok(ToolResult::text(text))
            }
            Err(e) => Ok(ToolResult::error(format!("MCP tool error: {}", e))),
        }
    }

    fn is_read_only(&self) -> bool {
        false
    }
}
