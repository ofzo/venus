use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct CronDeleteTool;

#[async_trait]
impl Tool for CronDeleteTool {
    fn name(&self) -> &str {
        "CronDelete"
    }

    fn description(&self) -> &str {
        "Delete a scheduled cron job by its ID."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The cron job ID to delete"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let scheduler = match ctx.cron_scheduler.as_ref() {
            Some(s) => s,
            None => return Ok(ToolResult::error("cron scheduler not available")),
        };

        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

        let removed = scheduler.delete_job(id).await?;

        if removed {
            Ok(ToolResult::text(format!("Deleted cron job {}", id)))
        } else {
            Ok(ToolResult::error(format!("No cron job found with id {}", id)))
        }
    }

    fn format_for_display(&self, input: &Value) -> String {
        let id = input.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        format!("CronDelete: {}", id)
    }
}
