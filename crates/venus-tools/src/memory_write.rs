use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use venus_core::tool::{Tool, ToolContext, ToolResult};
use venus_utils::memory::{self, MemoryEntry, MemoryType};

pub struct MemoryWriteTool;

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &str {
        "MemoryWrite"
    }

    fn description(&self) -> &str {
        "Create or update a persistent memory entry. Memories survive across sessions."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Memory ID to update. Omit to create new."
                },
                "memory_type": {
                    "type": "string",
                    "enum": ["user", "feedback", "project", "reference"],
                    "description": "Type of memory entry"
                },
                "title": {
                    "type": "string",
                    "description": "Short title for the memory"
                },
                "content": {
                    "type": "string",
                    "description": "Memory content"
                },
                "scope": {
                    "type": "string",
                    "enum": ["user", "project"],
                    "description": "Storage scope (default: user)"
                }
            },
            "required": ["memory_type", "title", "content"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let memory_type: MemoryType = input
            .get("memory_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing memory_type"))?
            .parse()?;

        let title = input
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing title"))?;

        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing content"))?;

        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let scope = input
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("user");

        let project_root = if scope == "project" {
            Some(ctx.working_dir.as_path())
        } else {
            None
        };

        // If updating an existing entry, preserve created_at
        let now = chrono::Utc::now().timestamp() as u64;
        let created_at = if let Ok(Some(existing)) = memory::read_memory(&id, project_root).await {
            existing.created_at
        } else {
            now
        };

        let entry = MemoryEntry {
            id: id.clone(),
            memory_type,
            title: title.to_string(),
            content: content.to_string(),
            created_at,
            updated_at: now,
        };

        memory::write_memory(&entry, project_root).await?;
        Ok(ToolResult::text(format!(
            "Memory saved: {} ({})",
            entry.title, entry.id
        )))
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn format_for_display(&self, input: &Value) -> String {
        let title = input
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        format!("MemoryWrite: {}", title)
    }
}
