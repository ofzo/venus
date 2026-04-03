use anyhow::Result;
use async_trait::async_trait;
use venus_core::tool::{Tool, ToolContext, ToolResult};
use venus_utils::fs_helpers;
use serde_json::Value;
use tokio::process::Command;

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> &str {
        "A search tool built on ripgrep. Supports regex patterns and file type filtering."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in"
                },
                "glob": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. \"*.js\")"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "Output mode (default: files_with_matches)"
                },
                "-i": {
                    "type": "boolean",
                    "description": "Case insensitive search"
                },
                "-A": {
                    "type": "number",
                    "description": "Lines to show after each match"
                },
                "-B": {
                    "type": "number",
                    "description": "Lines to show before each match"
                },
                "-C": {
                    "type": "number",
                    "description": "Context lines around each match"
                },
                "head_limit": {
                    "type": "number",
                    "description": "Max output entries (default: 250)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'pattern' parameter"))?;

        let search_path = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| fs_helpers::resolve_path(p, &ctx.working_dir))
            .unwrap_or_else(|| ctx.working_dir.clone());

        let output_mode = input
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files_with_matches");

        let head_limit = input
            .get("head_limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(250) as usize;

        let mut args: Vec<String> = vec!["--color=never".to_string()];

        match output_mode {
            "files_with_matches" => args.push("-l".to_string()),
            "count" => args.push("-c".to_string()),
            "content" => {
                args.push("-n".to_string());

                if let Some(a) = input.get("-A").and_then(|v| v.as_u64()) {
                    args.push(format!("-A{}", a));
                }
                if let Some(b) = input.get("-B").and_then(|v| v.as_u64()) {
                    args.push(format!("-B{}", b));
                }
                if let Some(c) = input.get("-C").and_then(|v| v.as_u64()) {
                    args.push(format!("-C{}", c));
                }
            }
            _ => args.push("-l".to_string()),
        }

        if input.get("-i").and_then(|v| v.as_bool()).unwrap_or(false) {
            args.push("-i".to_string());
        }

        if let Some(glob_pattern) = input.get("glob").and_then(|v| v.as_str()) {
            args.push("--glob".to_string());
            args.push(glob_pattern.to_string());
        }

        args.push("--".to_string());
        args.push(pattern.to_string());
        args.push(search_path.display().to_string());

        let output = Command::new("rg")
            .args(&args)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("failed to run rg: {} (is ripgrep installed?)", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        if stdout.is_empty() {
            return Ok(ToolResult::text("No matches found.".to_string()));
        }

        // Apply head_limit
        let lines: Vec<&str> = stdout.lines().collect();
        let truncated = lines.len() > head_limit;
        let limited: Vec<&str> = lines.into_iter().take(head_limit).collect();
        let mut result = limited.join("\n");

        if truncated {
            result.push_str(&format!("\n... (truncated to {} entries)", head_limit));
        }

        Ok(ToolResult::text(result))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
        format!("grep: {}", pattern)
    }
}
