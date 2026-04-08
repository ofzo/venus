use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct TaskStopTool;

#[async_trait]
impl Tool for TaskStopTool {
    fn name(&self) -> &str {
        "TaskStop"
    }

    fn description(&self) -> &str {
        "Cancel a running background task."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The background task ID to cancel"
                }
            },
            "required": ["task_id"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let task_id = input
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing task_id"))?;

        let cancelled = ctx.background_runtime.cancel(task_id).await?;
        if cancelled {
            Ok(ToolResult::text(format!("Task {} cancelled", task_id)))
        } else {
            Ok(ToolResult::text(format!(
                "Task {} not found or not running",
                task_id
            )))
        }
    }
}
