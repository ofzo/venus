use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn description(&self) -> &str {
        "Search the web using a search API. Requires API configuration."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;

        let message = format!(
            "Web search is not yet configured. No search API key has been provided.\n\n\
             Query: \"{}\"\n\n\
             To search the web, you can use the WebFetch tool with a specific URL instead. \
             For example, try fetching a search engine results page or a known website directly.\n\n\
             In the future, this tool will support search APIs (e.g., Brave Search, Google Custom Search) \
             once an API key is configured.",
            query
        );

        Ok(ToolResult::text(message))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let query = input.get("query").and_then(|v| v.as_str()).unwrap_or("?");
        format!("search: {}", query)
    }
}
