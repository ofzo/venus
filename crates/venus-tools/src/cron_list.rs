use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct CronListTool;

#[async_trait]
impl Tool for CronListTool {
    fn name(&self) -> &str {
        "CronList"
    }

    fn description(&self) -> &str {
        "List all scheduled cron jobs."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let scheduler = match ctx.cron_scheduler.as_ref() {
            Some(s) => s,
            None => return Ok(ToolResult::error("cron scheduler not available")),
        };

        let jobs = scheduler.list_jobs().await;

        if jobs.is_empty() {
            return Ok(ToolResult::text("No scheduled cron jobs."));
        }

        let mut output = String::new();
        output.push_str(&format!("{} scheduled job(s):\n\n", jobs.len()));

        for job in &jobs {
            output.push_str(&format!("- {} | cron: {} | recurring: {} | durable: {}\n", job.id, job.cron_expr, job.recurring, job.durable));
            output.push_str(&format!("  prompt: {}\n", job.prompt));
            if let Some(last) = job.last_fired {
                output.push_str(&format!("  last fired: {}\n", last));
            }
            output.push('\n');
        }

        Ok(ToolResult::text(output))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, _input: &Value) -> String {
        "CronList".to_string()
    }
}
