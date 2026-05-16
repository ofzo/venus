use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct WorkflowTool;

#[async_trait]
impl Tool for WorkflowTool {
    fn name(&self) -> &str {
        "Workflow"
    }

    fn description(&self) -> &str {
        "Execute or describe a workflow script. Workflows are JSON files that define a sequence of steps."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "workflow_path": {
                    "type": "string",
                    "description": "Path to the JSON workflow file"
                },
                "variables": {
                    "type": "object",
                    "description": "Optional variables to substitute into the workflow steps"
                }
            },
            "required": ["workflow_path"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let workflow_path = input
            .get("workflow_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'workflow_path' parameter"))?;

        let path = if Path::new(workflow_path).is_absolute() {
            PathBuf::from(workflow_path)
        } else {
            ctx.working_dir.join(workflow_path)
        };

        if !path.exists() {
            return Err(anyhow::anyhow!(
                "workflow file not found: {}",
                path.display()
            ));
        }

        let content = std::fs::read_to_string(&path)?;
        let workflow: Value = serde_json::from_str(&content).map_err(|e| {
            anyhow::anyhow!(
                "invalid JSON in workflow file {}: {}",
                path.display(),
                e
            )
        })?;

        let variables = input.get("variables");

        // Parse workflow structure
        let name = workflow
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("(unnamed workflow)");

        let description = workflow
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("(no description)");

        let steps = workflow
            .get("steps")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut result_lines = vec![
            format!("Workflow: {}", name),
            format!("Description: {}", description),
            format!("File: {}", path.display()),
            format!("Steps ({}):", steps.len()),
        ];

        for (i, step) in steps.iter().enumerate() {
            let step_name = step
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unnamed step");
            let step_action = step
                .get("action")
                .and_then(|v| v.as_str())
                .or_else(|| step.get("tool").and_then(|v| v.as_str()))
                .unwrap_or("unknown action");

            // Substitute variables if present
            let mut action = step_action.to_string();
            if let Some(vars) = variables {
                if let Some(obj) = vars.as_object() {
                    for (key, val) in obj {
                        let placeholder = format!("{{{{{}}}}}", key);
                        let val_str = match val {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        action = action.replace(&placeholder, &val_str);
                    }
                }
            }

            result_lines.push(format!("  {}. {} ({})", i + 1, step_name, action));
        }

        if let Some(vars) = variables {
            result_lines.push(format!(
                "Variables: {}",
                serde_json::to_string_pretty(vars).unwrap_or_default()
            ));
        }

        Ok(ToolResult::text(result_lines.join("\n")))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let path = input
            .get("workflow_path")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        format!("Workflow: {}", path)
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
    use venus_core::tool::{PermissionDecision, PermissionHandler};
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
            messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            auth_header: "",
            auth_value: String::new(),
            base_url: String::new(),
            model: String::new(),
            tools: Arc::new(ToolRegistry::new(vec![])),
            hook_runner: Arc::new(HookRunner::new(None, "test-session".to_string(), dir.to_path_buf())),
            cron_scheduler: None,
        }
    }

    #[tokio::test]
    async fn test_parse_valid_workflow() {
        let tmp = TempDir::new().unwrap();
        let wf_path = tmp.path().join("workflow.json");
        let workflow = serde_json::json!({
            "name": "Build Pipeline",
            "description": "Run build steps",
            "steps": [
                {"name": "Compile", "action": "cargo build"},
                {"name": "Test", "action": "cargo test"}
            ]
        });
        std::fs::write(&wf_path, serde_json::to_string_pretty(&workflow).unwrap()).unwrap();

        let ctx = make_context(tmp.path());
        let tool = WorkflowTool;
        let input = serde_json::json!({ "workflow_path": "workflow.json" });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Build Pipeline"));
        assert!(text.contains("Compile"));
        assert!(text.contains("Test"));
    }

    #[tokio::test]
    async fn test_workflow_not_found() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = WorkflowTool;
        let input = serde_json::json!({ "workflow_path": "nonexistent.json" });

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_workflow_variable_substitution() {
        let tmp = TempDir::new().unwrap();
        let wf_path = tmp.path().join("wf.json");
        let workflow = serde_json::json!({
            "name": "Deploy",
            "steps": [
                {"name": "Deploy", "action": "deploy {{env}} to {{region}}"}
            ]
        });
        std::fs::write(&wf_path, serde_json::to_string_pretty(&workflow).unwrap()).unwrap();

        let ctx = make_context(tmp.path());
        let tool = WorkflowTool;
        let input = serde_json::json!({
            "workflow_path": "wf.json",
            "variables": { "env": "prod", "region": "us-east-1" }
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("deploy prod to us-east-1"));
    }
}
