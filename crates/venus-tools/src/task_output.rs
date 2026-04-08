use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct TaskOutputTool;

#[async_trait]
impl Tool for TaskOutputTool {
    fn name(&self) -> &str {
        "TaskOutput"
    }

    fn description(&self) -> &str {
        "Retrieve output from a running or completed background task."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "The background task ID" },
                "block": { "type": "boolean", "description": "Whether to wait for completion (default: true)" },
                "timeout": { "type": "number", "description": "Max wait time in ms (default: 30000)" }
            },
            "required": ["task_id"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let task_id = input
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing task_id"))?;
        let block = input
            .get("block")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let timeout_ms = input
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(30000);

        if block {
            let deadline =
                tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);
            loop {
                let (info, output) = ctx.background_runtime.read_output(task_id).await?;
                if info.status != venus_core::background::BackgroundTaskStatus::Running {
                    return Ok(ToolResult::text(format!(
                        "Task {} ({:?}):\n{}",
                        task_id, info.status, output
                    )));
                }
                if tokio::time::Instant::now() >= deadline {
                    return Ok(ToolResult::text(format!(
                        "Task {} still running (timed out after {}ms). Current output:\n{}",
                        task_id, timeout_ms, output
                    )));
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        } else {
            let (info, output) = ctx.background_runtime.read_output(task_id).await?;
            Ok(ToolResult::text(format!(
                "Task {} ({:?}):\n{}",
                task_id, info.status, output
            )))
        }
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
