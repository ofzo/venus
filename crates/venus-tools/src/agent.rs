use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::subagent::{SubAgent, SubAgentConfig};
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct AgentTool;

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }

    fn description(&self) -> &str {
        "Launch a new agent to handle complex, multi-step tasks autonomously. \
         The sub-agent has its own conversation context and can use tools independently."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The task description for the sub-agent"
                },
                "description": {
                    "type": "string",
                    "description": "Short 3-5 word description of the task"
                },
                "subagent_type": {
                    "type": "string",
                    "enum": ["general-purpose", "Explore", "Plan"],
                    "description": "Type of sub-agent to spawn (default: general-purpose)"
                },
                "model": {
                    "type": "string",
                    "description": "Model override for the sub-agent"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Whether to run in background (default: false)"
                },
                "isolation": {
                    "type": "string",
                    "enum": ["worktree"],
                    "description": "Isolation mode for the sub-agent"
                }
            },
            "required": ["prompt", "description"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let prompt = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'prompt' parameter"))?;

        let description = input
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("sub-agent task");

        let run_in_background = input
            .get("run_in_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let model_override = input
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if run_in_background {
            let runtime = ctx.background_runtime.clone();
            let config = SubAgentConfig {
                prompt: prompt.to_string(),
                description: description.to_string(),
                model: model_override,
                working_dir: ctx.working_dir.clone(),
                auth_header: ctx.auth_header,
                auth_value: ctx.auth_value.clone(),
                base_url: ctx.base_url.clone(),
                settings: ctx.settings.clone(),
                tools: ctx.tools.clone(),
                permissions: ctx.permission_handler.clone(),
                task_store: ctx.task_store.clone(),
                background_runtime: ctx.background_runtime.clone(),
                hook_runner: ctx.hook_runner.clone(),
            };
            let task_id = runtime
                .spawn(description.to_string(), async move {
                    match SubAgent::run(config).await {
                        Ok(result) => Ok(result.output),
                        Err(e) => Err(e.to_string()),
                    }
                })
                .await;
            return Ok(ToolResult::text(format!(
                "Background agent started: {}. Use TaskOutput to check results.",
                task_id
            )));
        }

        let config = SubAgentConfig {
            prompt: prompt.to_string(),
            description: description.to_string(),
            model: model_override,
            working_dir: ctx.working_dir.clone(),
            auth_header: ctx.auth_header,
            auth_value: ctx.auth_value.clone(),
            base_url: ctx.base_url.clone(),
            settings: ctx.settings.clone(),
            tools: ctx.tools.clone(),
            permissions: ctx.permission_handler.clone(),
            task_store: ctx.task_store.clone(),
            background_runtime: ctx.background_runtime.clone(),
            hook_runner: ctx.hook_runner.clone(),
        };

        let result = SubAgent::run(config).await?;

        if result.is_error {
            Ok(ToolResult::error(result.output))
        } else {
            Ok(ToolResult::text(result.output))
        }
    }

    fn format_for_display(&self, input: &Value) -> String {
        let desc = input
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("sub-agent task");
        format!("Agent: {}", desc)
    }
}
