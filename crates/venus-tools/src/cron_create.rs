use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct CronCreateTool;

#[async_trait]
impl Tool for CronCreateTool {
    fn name(&self) -> &str {
        "CronCreate"
    }

    fn description(&self) -> &str {
        "Schedule a cron job that executes a prompt on a recurring or one-shot schedule. Uses standard 5-field cron expressions (minute hour day-of-month month day-of-week)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "cron": {
                    "type": "string",
                    "description": "5-field cron expression, e.g. '*/5 * * * *' for every 5 minutes"
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt to execute when the cron fires"
                },
                "recurring": {
                    "type": "boolean",
                    "description": "Whether the job repeats (default true)"
                },
                "durable": {
                    "type": "boolean",
                    "description": "Whether the job persists across sessions (default false)"
                }
            },
            "required": ["cron", "prompt"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let scheduler = match ctx.cron_scheduler.as_ref() {
            Some(s) => s,
            None => return Ok(ToolResult::error("cron scheduler not available")),
        };

        let cron_expr = input
            .get("cron")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'cron' parameter"))?
            .to_string();

        let prompt = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'prompt' parameter"))?
            .to_string();

        let recurring = input
            .get("recurring")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let durable = input
            .get("durable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let id = scheduler
            .create_job(cron_expr.clone(), prompt, recurring, durable)
            .await?;

        Ok(ToolResult::text(format!(
            "Scheduled job {} with cron '{}'",
            id, cron_expr
        )))
    }

    fn format_for_display(&self, input: &Value) -> String {
        let cron = input.get("cron").and_then(|v| v.as_str()).unwrap_or("?");
        format!("CronCreate: {}", cron)
    }
}
