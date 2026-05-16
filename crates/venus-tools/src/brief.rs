use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct BriefTool;

#[async_trait]
impl Tool for BriefTool {
    fn name(&self) -> &str {
        "Brief"
    }

    fn description(&self) -> &str {
        "Enable or disable brief response mode. When enabled, responses will be concise."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "brief": {"type": "boolean", "description": "Enable (true) or disable (false) brief mode"}
            },
            "required": ["brief"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let brief = input
            .get("brief")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if brief {
            Ok(ToolResult::text("Brief mode enabled. Responses will be concise."))
        } else {
            Ok(ToolResult::text(
                "Brief mode disabled. Responses will be detailed.",
            ))
        }
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_test_context;

    #[tokio::test]
    async fn test_brief_enable() {
        let tool = BriefTool;
        let ctx = make_test_context();
        let input = serde_json::json!({"brief": true});
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Brief mode enabled"));
    }

    #[tokio::test]
    async fn test_brief_disable() {
        let tool = BriefTool;
        let ctx = make_test_context();
        let input = serde_json::json!({"brief": false});
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Brief mode disabled"));
    }

    #[tokio::test]
    async fn test_brief_default_enabled() {
        let tool = BriefTool;
        let ctx = make_test_context();
        let input = serde_json::json!({});
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Brief mode enabled"));
    }
}
