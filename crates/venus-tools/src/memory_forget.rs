use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};
use venus_utils::memory;

pub struct MemoryForgetTool;

#[async_trait]
impl Tool for MemoryForgetTool {
    fn name(&self) -> &str {
        "MemoryForget"
    }

    fn description(&self) -> &str {
        "Delete a persistent memory entry by ID."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Memory ID to delete"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

        // Try project dir first, then user dir
        let deleted = memory::delete_memory(id, Some(ctx.working_dir.as_path())).await?;
        if deleted {
            return Ok(ToolResult::text(format!("Memory deleted: {}", id)));
        }

        // Try user-level (None project_root)
        let deleted = memory::delete_memory(id, None).await?;
        if deleted {
            return Ok(ToolResult::text(format!("Memory deleted: {}", id)));
        }

        Ok(ToolResult::text(format!("Memory not found: {}", id)))
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn format_for_display(&self, input: &Value) -> String {
        let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        format!("MemoryForget: {}", id)
    }
}
