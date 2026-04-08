use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use venus_core::task::{TaskStatus, TaskUpdate};
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct TaskUpdateTool;

#[async_trait]
impl Tool for TaskUpdateTool {
    fn name(&self) -> &str {
        "TaskUpdate"
    }

    fn description(&self) -> &str {
        "Update an existing task's status, subject, description, or other fields."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "taskId": {
                    "type": "string",
                    "description": "The ID of the task to update"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed", "deleted"],
                    "description": "New status for the task"
                },
                "subject": {
                    "type": "string",
                    "description": "Updated subject"
                },
                "description": {
                    "type": "string",
                    "description": "Updated description"
                },
                "activeForm": {
                    "type": "string",
                    "description": "Updated active form label"
                },
                "owner": {
                    "type": "string",
                    "description": "Updated owner"
                },
                "addBlocks": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs that this task blocks"
                },
                "addBlockedBy": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs that block this task"
                },
                "metadata": {
                    "type": "object",
                    "description": "Additional key-value metadata to merge"
                }
            },
            "required": ["taskId"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let task_id = input
            .get("taskId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'taskId' parameter"))?;

        let status = input
            .get("status")
            .and_then(|v| v.as_str())
            .map(parse_status)
            .transpose()?;

        let subject = input
            .get("subject")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let description = input
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let active_form = input
            .get("activeForm")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let owner = input
            .get("owner")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let add_blocks = input.get("addBlocks").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect()
            })
        });

        let add_blocked_by = input.get("addBlockedBy").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect()
            })
        });

        let metadata: Option<HashMap<String, serde_json::Value>> =
            input.get("metadata").and_then(|v| {
                v.as_object().map(|obj| {
                    obj.iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
            });

        let updates = TaskUpdate {
            status,
            subject,
            description,
            active_form,
            owner,
            add_blocks,
            add_blocked_by,
            metadata,
        };

        match ctx.task_store.update(task_id, updates) {
            Some(task) => {
                let json = serde_json::to_string_pretty(&task)?;
                Ok(ToolResult::text(json))
            }
            None => Ok(ToolResult::error(format!("task not found: {}", task_id))),
        }
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let id = input.get("taskId").and_then(|v| v.as_str()).unwrap_or("?");
        format!("TaskUpdate: {}", id)
    }
}

fn parse_status(s: &str) -> Result<TaskStatus> {
    match s {
        "pending" => Ok(TaskStatus::Pending),
        "in_progress" => Ok(TaskStatus::InProgress),
        "completed" => Ok(TaskStatus::Completed),
        "deleted" => Ok(TaskStatus::Deleted),
        other => Err(anyhow::anyhow!("invalid status: {}", other)),
    }
}
