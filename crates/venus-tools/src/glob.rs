use anyhow::Result;
use async_trait::async_trait;
use venus_core::tool::{Tool, ToolContext, ToolResult};
use venus_utils::fs_helpers;
use globset::{Glob as GlobPattern, GlobSetBuilder};
use serde_json::Value;
use std::path::PathBuf;

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn description(&self) -> &str {
        "Fast file pattern matching tool. Returns matching file paths sorted by modification time."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern to match files against"
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in (default: working directory)"
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

        let search_dir = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| fs_helpers::resolve_path(p, &ctx.working_dir))
            .unwrap_or_else(|| ctx.working_dir.clone());

        if !search_dir.exists() {
            return Ok(ToolResult::error(format!(
                "directory not found: {}",
                search_dir.display()
            )));
        }

        let glob = GlobPattern::new(pattern)
            .map_err(|e| anyhow::anyhow!("invalid glob pattern '{}': {}", pattern, e))?;

        let mut builder = GlobSetBuilder::new();
        builder.add(glob);
        let glob_set = builder.build()?;

        // Use ignore crate to respect .gitignore
        let walker = ignore::WalkBuilder::new(&search_dir)
            .hidden(false)
            .git_ignore(true)
            .build();

        let mut matches: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        let limit = 100;

        for entry in walker.flatten() {
            if matches.len() >= limit {
                break;
            }

            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Match relative path against glob
            let rel_path = path.strip_prefix(&search_dir).unwrap_or(path);
            if glob_set.is_match(rel_path) || glob_set.is_match(path) {
                let mtime = path
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                matches.push((path.to_path_buf(), mtime));
            }
        }

        // Sort by modification time (newest first)
        matches.sort_by(|a, b| b.1.cmp(&a.1));

        if matches.is_empty() {
            return Ok(ToolResult::text(format!(
                "No files matching '{}' in {}",
                pattern,
                search_dir.display()
            )));
        }

        let output: Vec<String> = matches.iter().map(|(p, _)| p.display().to_string()).collect();
        let total = output.len();
        let result = output.join("\n");

        Ok(ToolResult::text(format!(
            "{} file(s) found:\n{}",
            total, result
        )))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
        format!("glob: {}", pattern)
    }
}
