use anyhow::Result;
use async_trait::async_trait;
use venus_core::tool::{Tool, ToolContext, ToolResult};
use venus_utils::fs_helpers;
use serde_json::Value;

pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn description(&self) -> &str {
        "Reads a file from the local filesystem."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to read"
                },
                "offset": {
                    "type": "number",
                    "description": "Line number to start reading from (1-based)"
                },
                "limit": {
                    "type": "number",
                    "description": "Number of lines to read"
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let file_path = input
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'file_path' parameter"))?;

        let path = fs_helpers::resolve_path(file_path, &ctx.working_dir);

        if !path.exists() {
            return Ok(ToolResult::error(format!(
                "file not found: {}",
                path.display()
            )));
        }

        if !path.is_file() {
            return Ok(ToolResult::error(format!(
                "not a file: {}",
                path.display()
            )));
        }

        let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
            anyhow::anyhow!("failed to read {}: {}", path.display(), e)
        })?;

        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(2000) as usize;

        let lines: Vec<&str> = content.lines().collect();
        let start = (offset.saturating_sub(1)).min(lines.len());
        let end = (start + limit).min(lines.len());

        let numbered: String = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>6}\t{}", start + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        let header = format!(
            "File: {} ({} lines total)\n",
            path.display(),
            lines.len()
        );

        Ok(ToolResult::text(format!("{}{}", header, numbered)))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let path = input
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        format!("read: {}", path)
    }
}
