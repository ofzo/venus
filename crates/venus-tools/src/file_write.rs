use anyhow::Result;
use async_trait::async_trait;
use venus_core::tool::{Tool, ToolContext, ToolResult};
use venus_utils::diff::{compute_file_diff, compute_new_file_diff, ToolDiff};
use venus_utils::fs_helpers;
use serde_json::Value;

pub struct FileWriteTool;

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn description(&self) -> &str {
        "Writes a file to the local filesystem. Creates parent directories as needed."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let file_path = input
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'file_path' parameter"))?;
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'content' parameter"))?;

        let path = fs_helpers::resolve_path(file_path, &ctx.working_dir);

        // Create parent directories
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Read the previous content (if any) so we can attach a structured
        // diff to the tool result for the TUI's colourised write preview.
        let old_content = if path.exists() {
            tokio::fs::read_to_string(&path).await.ok()
        } else {
            None
        };
        let is_new = old_content.is_none();

        tokio::fs::write(&path, content).await?;

        let line_count = content.lines().count();
        let action = if is_new { "Created" } else { "Wrote" };

        // Compute a structured diff (old vs new) so the conversation view can
        // render a Claude-Code-style `+/-` preview. For brand-new files we
        // produce an all-add hunk.
        let diff_lines = match &old_content {
            Some(old) => compute_file_diff(old, content),
            None => compute_new_file_diff(content),
        };
        let diff = ToolDiff::new(path.display().to_string(), diff_lines);

        Ok(ToolResult::text(format!(
            "{} {} ({} lines)",
            action,
            path.display(),
            line_count
        ))
        .with_diff(diff))
    }

    fn format_for_display(&self, input: &Value) -> String {
        let path = input
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        format!("write: {}", path)
    }
}
