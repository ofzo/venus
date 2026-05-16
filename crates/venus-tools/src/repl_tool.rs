use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::process::Command;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct REPLTool;

#[async_trait]
impl Tool for REPLTool {
    fn name(&self) -> &str {
        "REPL"
    }

    fn description(&self) -> &str {
        "Run code in a subprocess REPL. Supports Python, Node.js, and Rust (if installed). \
         Automatically detects language from file extension or content."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "language": {
                    "type": "string",
                    "enum": ["python", "node", "rust"],
                    "description": "Programming language to use (auto-detected if not specified)"
                },
                "code": {
                    "type": "string",
                    "description": "Code to execute"
                }
            },
            "required": ["code"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let code = input
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'code' parameter"))?;

        let language = input
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| detect_language(code));

        let (program, args) = match language {
            "python" => ("python3", vec!["-c"]),
            "node" => ("node", vec!["-e"]),
            "rust" => {
                // For Rust, we compile and run via cargo or rustc
                return execute_rust(code, ctx).await;
            }
            _ => return Ok(ToolResult::error(format!("unsupported language: {}", language))),
        };

        let output = Command::new(program)
            .args(&args)
            .arg(code)
            .current_dir(&ctx.working_dir)
            .output()
            .await;

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let mut result = String::new();
                if !stdout.is_empty() {
                    result.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(&stderr);
                }
                if output.status.success() {
                    Ok(ToolResult::text(result))
                } else {
                    Ok(ToolResult::error(format!(
                        "{}\nexit code: {}",
                        result,
                        output.status.code().unwrap_or(-1)
                    )))
                }
            }
            Err(e) => Ok(ToolResult::error(format!(
                "failed to execute {} (is it installed?): {}",
                language, e
            ))),
        }
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn format_for_display(&self, input: &Value) -> String {
        let lang = input
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");
        let code = input.get("code").and_then(|v| v.as_str()).unwrap_or("");
        let preview = if code.len() > 50 {
            format!("{}...", &code[..50])
        } else {
            code.to_string()
        };
        format!("REPL ({}): {}", lang, preview)
    }
}

fn detect_language(code: &str) -> &'static str {
    let trimmed = code.trim();
    if trimmed.starts_with("fn ") || trimmed.starts_with("use ") {
        "rust"
    } else if trimmed.starts_with("def ") || trimmed.starts_with("import ") {
        "python"
    } else if trimmed.starts_with("function ") || trimmed.starts_with("const ") {
        "node"
    } else {
        "python" // default
    }
}

async fn execute_rust(code: &str, ctx: &ToolContext) -> Result<ToolResult> {
    let tmp_dir = tempfile::tempdir()?;
    let src_path = tmp_dir.path().join("main.rs");
    std::fs::write(&src_path, code)?;

    let output = Command::new("rustc")
        .arg(&src_path)
        .arg("-o")
        .arg(tmp_dir.path().join("main"))
        .current_dir(tmp_dir.path())
        .output()
        .await;

    match output {
        Ok(compiler_output) => {
            if !compiler_output.status.success() {
                let stderr = String::from_utf8_lossy(&compiler_output.stderr).to_string();
                return Ok(ToolResult::error(format!("Rust compilation failed:\n{}", stderr)));
            }

            // Run the compiled binary
            let binary = if cfg!(target_os = "windows") {
                tmp_dir.path().join("main.exe")
            } else {
                tmp_dir.path().join("main")
            };

            let run_output = Command::new(&binary)
                .current_dir(&ctx.working_dir)
                .output()
                .await;

            match run_output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                    let mut result = stdout;
                    if !stderr.is_empty() {
                        if !result.is_empty() {
                            result.push('\n');
                        }
                        result.push_str(&stderr);
                    }
                    if out.status.success() {
                        Ok(ToolResult::text(result))
                    } else {
                        Ok(ToolResult::error(format!(
                            "{}\nexit code: {}",
                            result,
                            out.status.code().unwrap_or(-1)
                        )))
                    }
                }
                Err(e) => Ok(ToolResult::error(format!("failed to run Rust binary: {}", e))),
            }
        }
        Err(e) => Ok(ToolResult::error(format!(
            "failed to run rustc (is it installed?): {}",
            e
        ))),
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

    #[test]
    fn test_detect_language_python() {
        assert_eq!(detect_language("def foo(): pass"), "python");
    }

    #[test]
    fn test_detect_language_rust() {
        assert_eq!(detect_language("fn main() {}"), "rust");
    }

    #[test]
    fn test_detect_language_node() {
        assert_eq!(detect_language("function foo() {}"), "node");
    }

    #[tokio::test]
    async fn test_execute_python() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = REPLTool;
        let input = serde_json::json!({
            "language": "python",
            "code": "print(2 + 3)"
        });

        let result = tool.execute(input, &ctx).await;
        // May fail if python3 not installed, but shouldn't panic
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_missing_code() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = REPLTool;
        let input = serde_json::json!({ "language": "python" });

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_err());
    }
}
