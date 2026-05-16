use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct TodoWriteTool;

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "TodoWrite"
    }

    fn description(&self) -> &str {
        "Manage a session-level todo checklist. Write the full list of todos each time."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "content": {"type": "string"},
                            "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]}
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let todos = input.get("todos").and_then(|v| v.as_array());
        match todos {
            Some(items) => {
                let mut output = String::from("Todo list updated:\n");
                for item in items {
                    let content = item
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let status = item
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("pending");
                    let icon = match status {
                        "completed" => "x",
                        "in_progress" => ">",
                        _ => " ",
                    };
                    output.push_str(&format!("  [{}] {}\n", icon, content));
                }
                Ok(ToolResult::text(output))
            }
            None => Ok(ToolResult::error("Missing 'todos' array")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_test_context;

    #[tokio::test]
    async fn test_todo_write_with_items() {
        let tool = TodoWriteTool;
        let ctx = make_test_context();
        let input = serde_json::json!({
            "todos": [
                {"content": "Implement feature X", "status": "in_progress"},
                {"content": "Write tests", "status": "pending"},
                {"content": "Fix bug Y", "status": "completed"}
            ]
        });
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("[>] Implement feature X"));
        assert!(text.contains("[ ] Write tests"));
        assert!(text.contains("[x] Fix bug Y"));
    }

    #[tokio::test]
    async fn test_todo_write_empty_list() {
        let tool = TodoWriteTool;
        let ctx = make_test_context();
        let input = serde_json::json!({"todos": []});
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Todo list updated:"));
    }

    #[tokio::test]
    async fn test_todo_write_missing_todos() {
        let tool = TodoWriteTool;
        let ctx = make_test_context();
        let input = serde_json::json!({});
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Missing 'todos' array"));
    }
}
