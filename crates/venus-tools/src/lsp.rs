use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
use venus_core::lsp::{LspManager, LspResult};
use venus_core::tool::{Tool, ToolContext, ToolResult};
use venus_utils::fs_helpers::resolve_path;

/// Tool that provides code intelligence via Language Server Protocol.
pub struct LspTool {
    manager: Arc<Mutex<Option<LspManager>>>,
}

impl LspTool {
    pub fn new() -> Self {
        Self {
            manager: Arc::new(Mutex::new(None)),
        }
    }

    async fn get_manager(&self, working_dir: &std::path::Path) -> Arc<Mutex<Option<LspManager>>> {
        let mut guard = self.manager.lock().await;
        if guard.is_none() {
            *guard = Some(LspManager::new(working_dir.to_path_buf()));
        }
        Arc::clone(&self.manager)
    }
}

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &str {
        "LSP"
    }

    fn description(&self) -> &str {
        "Interact with Language Server Protocol servers to get code intelligence features like go-to-definition, find-references, hover information, and symbol search."
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "The LSP operation to perform",
                    "enum": [
                        "goToDefinition",
                        "findReferences",
                        "hover",
                        "documentSymbol",
                        "workspaceSymbol",
                        "goToImplementation"
                    ]
                },
                "filePath": {
                    "type": "string",
                    "description": "Path to the file (absolute or relative to working directory)"
                },
                "line": {
                    "type": "number",
                    "description": "Line number (1-based)"
                },
                "character": {
                    "type": "number",
                    "description": "Character offset (1-based)"
                }
            },
            "required": ["operation", "filePath", "line", "character"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let operation = input
            .get("operation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'operation' parameter"))?;

        let file_path = input
            .get("filePath")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'filePath' parameter"))?;

        let line = input
            .get("line")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("missing 'line' parameter"))? as u32;

        let character = input
            .get("character")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("missing 'character' parameter"))? as u32;

        let resolved = resolve_path(file_path, &ctx.working_dir);

        // Ensure manager is initialized
        let mgr = self.get_manager(&ctx.working_dir).await;
        let guard = mgr.lock().await;
        let manager = guard.as_ref().unwrap();

        match manager.execute(operation, &resolved, line, character).await {
            Ok(result) => Ok(ToolResult::text(format_lsp_result(&result))),
            Err(e) => Ok(ToolResult::error(format!("LSP error: {e}"))),
        }
    }

    fn format_for_display(&self, input: &Value) -> String {
        let op = input.get("operation").and_then(|v| v.as_str()).unwrap_or("?");
        let file = input.get("filePath").and_then(|v| v.as_str()).unwrap_or("?");
        let line = input.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
        let char = input.get("character").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("LSP {op} {file}:{line}:{char}")
    }
}

fn format_lsp_result(result: &LspResult) -> String {
    match result {
        LspResult::Locations(locs) if locs.is_empty() => "No results found.".to_string(),
        LspResult::Locations(locs) => {
            let mut out = String::new();
            for loc in locs {
                out.push_str(&format!("{}:{}:{}", loc.file, loc.line, loc.character));
                if let (Some(el), Some(ec)) = (loc.end_line, loc.end_character) {
                    out.push_str(&format!("-{}:{}", el, ec));
                }
                out.push('\n');
            }
            out.trim_end().to_string()
        }
        LspResult::Hover(text) => text.clone(),
        LspResult::Symbols(syms) if syms.is_empty() => "No symbols found.".to_string(),
        LspResult::Symbols(syms) => {
            let mut out = String::new();
            for sym in syms {
                out.push_str(&format!(
                    "{} ({}) {}:{}:{}",
                    sym.name, sym.kind, sym.location.file, sym.location.line, sym.location.character,
                ));
                out.push('\n');
            }
            out.trim_end().to_string()
        }
        LspResult::Error(msg) => format!("Error: {msg}"),
    }
}
