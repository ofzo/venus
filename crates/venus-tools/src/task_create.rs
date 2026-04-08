use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct TaskCreateTool;

#[async_trait]
impl Tool for TaskCreateTool {
    fn name(&self) -> &str {
        "TaskCreate"
    }

    fn description(&self) -> &str {
        "Create a new task to track work. Returns the created task with its assigned ID."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subject": {
                    "type": "string",
                    "description": "Short summary of the task"
                },
                "description": {
                    "type": "string",
                    "description": "Detailed description of the task"
                },
                "activeForm": {
                    "type": "string",
                    "description": "Optional active form / working state label"
                }
            },
            "required": ["subject", "description"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let subject = input
            .get("subject")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'subject' parameter"))?
            .to_string();

        let description = input
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'description' parameter"))?
            .to_string();

        let active_form = input
            .get("activeForm")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let task = ctx.task_store.create(subject, description, active_form);
        let json = serde_json::to_string_pretty(&task)?;
        Ok(ToolResult::text(json))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let subject = input.get("subject").and_then(|v| v.as_str()).unwrap_or("?");
        format!("TaskCreate: {}", subject)
    }
}
