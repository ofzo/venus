use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct TaskListTool;

#[async_trait]
impl Tool for TaskListTool {
    fn name(&self) -> &str {
        "TaskList"
    }

    fn description(&self) -> &str {
        "List all non-deleted tasks."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let tasks = ctx.task_store.list();

        if tasks.is_empty() {
            return Ok(ToolResult::text("No tasks found."));
        }

        let json = serde_json::to_string_pretty(&tasks)?;
        Ok(ToolResult::text(json))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, _input: &Value) -> String {
        "TaskList".to_string()
    }
}
