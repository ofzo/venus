use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct TaskGetTool;

#[async_trait]
impl Tool for TaskGetTool {
    fn name(&self) -> &str {
        "TaskGet"
    }

    fn description(&self) -> &str {
        "Get full details of a task by its ID."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "taskId": {
                    "type": "string",
                    "description": "The ID of the task to retrieve"
                }
            },
            "required": ["taskId"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let task_id = input
            .get("taskId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'taskId' parameter"))?;

        match ctx.task_store.get(task_id) {
            Some(task) => {
                let json = serde_json::to_string_pretty(&task)?;
                Ok(ToolResult::text(json))
            }
            None => Ok(ToolResult::error(format!("task not found: {}", task_id))),
        }
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let id = input.get("taskId").and_then(|v| v.as_str()).unwrap_or("?");
        format!("TaskGet: {}", id)
    }
}
