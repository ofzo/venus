use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};
use venus_utils::memory::{self, MemoryType};

pub struct MemoryReadTool;

#[async_trait]
impl Tool for MemoryReadTool {
    fn name(&self) -> &str {
        "MemoryRead"
    }

    fn description(&self) -> &str {
        "Read persistent memory entries. Retrieve a specific memory by ID or list all memories filtered by type."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Memory ID to read. If omitted, lists all memories."
                },
                "memory_type": {
                    "type": "string",
                    "enum": ["user", "feedback", "project", "reference"],
                    "description": "Filter by memory type when listing"
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let project_root = Some(ctx.working_dir.as_path());

        // If ID is given, read a single memory
        if let Some(id) = input.get("id").and_then(|v| v.as_str()) {
            return match memory::read_memory(id, project_root).await? {
                Some(entry) => Ok(ToolResult::text(format_entry(&entry))),
                None => Ok(ToolResult::text(format!("Memory not found: {}", id))),
            };
        }

        // Otherwise, list memories with optional type filter
        let memory_type: Option<MemoryType> = input
            .get("memory_type")
            .and_then(|v| v.as_str())
            .map(|s| s.parse())
            .transpose()?;

        let entries = memory::list_memories(memory_type, project_root).await?;

        if entries.is_empty() {
            return Ok(ToolResult::text("No memories found."));
        }

        let mut output = format!("Found {} memories:\n\n", entries.len());
        for entry in &entries {
            output.push_str(&format!(
                "- [{}] **{}** (type: {}, id: {})\n",
                format_timestamp(entry.updated_at),
                entry.title,
                entry.memory_type,
                entry.id,
            ));
        }

        Ok(ToolResult::text(output))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        if let Some(id) = input.get("id").and_then(|v| v.as_str()) {
            format!("MemoryRead: {}", id)
        } else {
            "MemoryRead: list all".to_string()
        }
    }
}

fn format_entry(entry: &memory::MemoryEntry) -> String {
    format!(
        "# {}\n\nID: {}\nType: {}\nCreated: {}\nUpdated: {}\n\n{}",
        entry.title,
        entry.id,
        entry.memory_type,
        format_timestamp(entry.created_at),
        format_timestamp(entry.updated_at),
        entry.content,
    )
}

fn format_timestamp(ts: u64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}
