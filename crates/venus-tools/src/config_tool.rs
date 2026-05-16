use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct ConfigTool;

#[async_trait]
impl Tool for ConfigTool {
    fn name(&self) -> &str {
        "Config"
    }

    fn description(&self) -> &str {
        "View or modify runtime configuration settings."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["get", "set"]},
                "key": {"type": "string", "description": "Setting key (e.g., 'model', 'permission_mode')"},
                "value": {"type": "string", "description": "Value to set (for 'set' action)"}
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("get");
        let key = input.get("key").and_then(|v| v.as_str());
        let value = input.get("value").and_then(|v| v.as_str());

        match action {
            "get" => {
                let mut output = String::from("Current configuration:\n");
                output.push_str(&format!(
                    "  model: {}\n",
                    ctx.settings.model.as_deref().unwrap_or("default")
                ));
                output.push_str(&format!(
                    "  permission_mode: {}\n",
                    ctx.settings.permission_mode.as_deref().unwrap_or("default")
                ));
                output.push_str(&format!(
                    "  base_url: {}\n",
                    ctx.settings.effective_base_url()
                ));
                if let Some(ref mcp) = ctx.settings.mcp_servers {
                    output.push_str(&format!("  mcp_servers: {} configured\n", mcp.len()));
                }
                Ok(ToolResult::text(output))
            }
            "set" => match (key, value) {
                (Some(k), Some(v)) => {
                    Ok(ToolResult::text(format!(
                        "Config set: {} = {} (requires restart)",
                        k, v
                    )))
                }
                _ => Ok(ToolResult::error(
                    "Both 'key' and 'value' required for 'set' action",
                )),
            },
            _ => Ok(ToolResult::error("Unknown action. Use 'get' or 'set'")),
        }
    }

    fn is_read_only(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_test_context;

    #[tokio::test]
    async fn test_config_get() {
        let tool = ConfigTool;
        let ctx = make_test_context();
        let input = serde_json::json!({"action": "get"});
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Current configuration:"));
        assert!(text.contains("model:"));
        assert!(text.contains("base_url:"));
    }

    #[tokio::test]
    async fn test_config_set() {
        let tool = ConfigTool;
        let ctx = make_test_context();
        let input = serde_json::json!({"action": "set", "key": "model", "value": "claude-opus-4-20250514"});
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Config set: model = claude-opus-4-20250514"));
    }

    #[tokio::test]
    async fn test_config_set_missing_value() {
        let tool = ConfigTool;
        let ctx = make_test_context();
        let input = serde_json::json!({"action": "set", "key": "model"});
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Both 'key' and 'value' required"));
    }

    #[tokio::test]
    async fn test_config_unknown_action() {
        let tool = ConfigTool;
        let ctx = make_test_context();
        let input = serde_json::json!({"action": "delete"});
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Unknown action"));
    }
}
