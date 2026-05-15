use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct NotebookEditTool;

#[async_trait]
impl Tool for NotebookEditTool {
    fn name(&self) -> &str {
        "NotebookEdit"
    }

    fn description(&self) -> &str {
        "Edit cells in a Jupyter notebook (.ipynb file). Supports replace, insert, and delete operations."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "notebook_path": {
                    "type": "string",
                    "description": "Absolute path to the .ipynb file"
                },
                "cell_id": {
                    "type": "integer",
                    "description": "0-based index of the cell to edit"
                },
                "new_source": {
                    "type": "string",
                    "description": "New source content for the cell"
                },
                "cell_type": {
                    "type": "string",
                    "enum": ["code", "markdown"],
                    "description": "Cell type (default: code)"
                },
                "edit_mode": {
                    "type": "string",
                    "enum": ["replace", "insert", "delete"],
                    "description": "Edit mode: replace, insert, or delete (default: replace)"
                }
            },
            "required": ["notebook_path", "new_source"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let notebook_path = input
            .get("notebook_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'notebook_path' parameter"))?;

        let new_source = input
            .get("new_source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'new_source' parameter"))?;

        let path = if std::path::Path::new(notebook_path).is_absolute() {
            PathBuf::from(notebook_path)
        } else {
            ctx.working_dir.join(notebook_path)
        };

        if !path.exists() {
            return Err(anyhow::anyhow!("notebook file not found: {}", path.display()));
        }

        let content = std::fs::read_to_string(&path)?;
        let mut notebook: Value = serde_json::from_str(&content)?;

        let cells = notebook
            .get_mut("cells")
            .and_then(|v| v.as_array_mut())
            .ok_or_else(|| anyhow::anyhow!("invalid notebook: missing 'cells' array"))?;

        let cell_id = input.get("cell_id").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let edit_mode = input
            .get("edit_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("replace");
        let cell_type = input
            .get("cell_type")
            .and_then(|v| v.as_str())
            .unwrap_or("code");

        let source_lines: Vec<String> = new_source.split('\n').map(|s| s.to_string()).collect();
        let new_cell = serde_json::json!({
            "cell_type": cell_type,
            "source": source_lines,
            "metadata": {},
            "outputs": if cell_type == "code" { serde_json::json!([]) } else { Value::Null },
            "execution_count": if cell_type == "code" { Value::Null } else { Value::Null },
        });

        match edit_mode {
            "replace" => {
                if cell_id >= cells.len() {
                    return Err(anyhow::anyhow!(
                        "cell index {} out of range (notebook has {} cells)",
                        cell_id,
                        cells.len()
                    ));
                }
                cells[cell_id] = new_cell;
            }
            "insert" => {
                if cell_id > cells.len() {
                    return Err(anyhow::anyhow!(
                        "cell index {} out of range for insert (notebook has {} cells)",
                        cell_id,
                        cells.len()
                    ));
                }
                cells.insert(cell_id, new_cell);
            }
            "delete" => {
                if cell_id >= cells.len() {
                    return Err(anyhow::anyhow!(
                        "cell index {} out of range for delete (notebook has {} cells)",
                        cell_id,
                        cells.len()
                    ));
                }
                cells.remove(cell_id);
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "invalid edit_mode: {} (expected replace, insert, or delete)",
                    edit_mode
                ));
            }
        }

        let updated = serde_json::to_string_pretty(&notebook)?;
        std::fs::write(&path, updated)?;

        Ok(ToolResult::text(format!(
            "Notebook {} cell {} (mode: {})",
            path.display(),
            cell_id,
            edit_mode
        )))
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn format_for_display(&self, input: &Value) -> String {
        let path = input.get("notebook_path").and_then(|v| v.as_str()).unwrap_or("?");
        let mode = input.get("edit_mode").and_then(|v| v.as_str()).unwrap_or("replace");
        format!("NotebookEdit ({}): {}", mode, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;
    use venus_core::background::BackgroundTaskRuntime;
    use venus_core::hooks::HookRunner;
    use venus_core::task::TaskStore;
    use venus_core::tool::{PermissionHandler, PermissionDecision};
    use venus_core::tool_registry::ToolRegistry;
    use venus_utils::config::Settings;

    struct NoopPermission;
    #[async_trait]
    impl PermissionHandler for NoopPermission {
        async fn check_permission(&self, _: &str, _: &Value) -> PermissionDecision {
            PermissionDecision::Allow
        }
    }

    fn make_context(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            working_dir: dir.to_path_buf(),
            session_id: "test-session".to_string(),
            cancel_token: CancellationToken::new(),
            permission_handler: Arc::new(NoopPermission),
            settings: Arc::new(Settings::default()),
            task_store: Arc::new(TaskStore::new()),
            background_runtime: Arc::new(BackgroundTaskRuntime::new()),
            plan_mode: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            auth_header: "",
            auth_value: String::new(),
            base_url: String::new(),
            model: String::new(),
            tools: Arc::new(ToolRegistry::new(vec![])),
            hook_runner: Arc::new(HookRunner::new(None, "test-session".to_string(), dir.to_path_buf())),
            cron_scheduler: None,
        }
    }

    fn make_notebook(path: &std::path::Path, cells: Vec<Value>) {
        let nb = serde_json::json!({
            "cells": cells,
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        });
        std::fs::write(path, serde_json::to_string_pretty(&nb).unwrap()).unwrap();
    }

    fn code_cell(source: &str) -> Value {
        serde_json::json!({
            "cell_type": "code",
            "source": [source],
            "metadata": {},
            "outputs": [],
            "execution_count": null
        })
    }

    #[tokio::test]
    async fn test_replace_cell() {
        let tmp = TempDir::new().unwrap();
        let nb_path = tmp.path().join("test.ipynb");
        make_notebook(&nb_path, vec![code_cell("old code")]);

        let ctx = make_context(tmp.path());
        let tool = NotebookEditTool;
        let input = serde_json::json!({
            "notebook_path": nb_path.to_str().unwrap(),
            "cell_id": 0,
            "new_source": "new code",
            "edit_mode": "replace"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);

        let content = std::fs::read_to_string(&nb_path).unwrap();
        let nb: Value = serde_json::from_str(&content).unwrap();
        let cell_source = nb["cells"][0]["source"][0].as_str().unwrap();
        assert_eq!(cell_source, "new code");
    }

    #[tokio::test]
    async fn test_insert_and_delete_cell() {
        let tmp = TempDir::new().unwrap();
        let nb_path = tmp.path().join("test.ipynb");
        make_notebook(&nb_path, vec![code_cell("cell0"), code_cell("cell1")]);

        let ctx = make_context(tmp.path());
        let tool = NotebookEditTool;

        // Insert at index 1
        let input = serde_json::json!({
            "notebook_path": nb_path.to_str().unwrap(),
            "cell_id": 1,
            "new_source": "inserted",
            "edit_mode": "insert",
            "cell_type": "markdown"
        });
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);

        // Verify insert
        let content = std::fs::read_to_string(&nb_path).unwrap();
        let nb: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(nb["cells"].as_array().unwrap().len(), 3);
        assert_eq!(nb["cells"][1]["cell_type"], "markdown");

        // Delete index 1
        let input = serde_json::json!({
            "notebook_path": nb_path.to_str().unwrap(),
            "cell_id": 1,
            "new_source": "",
            "edit_mode": "delete"
        });
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);

        let content = std::fs::read_to_string(&nb_path).unwrap();
        let nb: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(nb["cells"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_cell_out_of_range() {
        let tmp = TempDir::new().unwrap();
        let nb_path = tmp.path().join("test.ipynb");
        make_notebook(&nb_path, vec![code_cell("cell0")]);

        let ctx = make_context(tmp.path());
        let tool = NotebookEditTool;
        let input = serde_json::json!({
            "notebook_path": nb_path.to_str().unwrap(),
            "cell_id": 5,
            "new_source": "x",
            "edit_mode": "replace"
        });

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_err());
    }
}
