use anyhow::Result;
use async_trait::async_trait;
use venus_core::tool::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Executes a given bash command and returns its output."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Optional timeout in milliseconds (max 600000)"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Whether to run in background (default: false)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'command' parameter"))?;

        let run_in_background = input
            .get("run_in_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let timeout_ms = input
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(120_000);

        if run_in_background {
            let runtime = ctx.background_runtime.clone();
            let command = command.to_string();
            let working_dir = ctx.working_dir.clone();
            let timeout = std::time::Duration::from_millis(timeout_ms.min(600_000));
            let task_id = runtime
                .spawn(format!("$ {}", &command), async move {
                    let mut child = Command::new("bash")
                        .arg("-c")
                        .arg(&command)
                        .current_dir(&working_dir)
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                        .map_err(|e| format!("failed to spawn: {}", e))?;

                    let result = tokio::time::timeout(timeout, async {
                        let mut stdout = String::new();
                        let mut stderr = String::new();
                        if let Some(mut out) = child.stdout.take() {
                            out.read_to_string(&mut stdout).await.ok();
                        }
                        if let Some(mut err) = child.stderr.take() {
                            err.read_to_string(&mut stderr).await.ok();
                        }
                        let status = child.wait().await.map_err(|e| e.to_string())?;
                        Ok::<_, String>((stdout, stderr, status))
                    })
                    .await;

                    match result {
                        Ok(Ok((stdout, stderr, status))) => {
                            let mut output = stdout;
                            if !stderr.is_empty() {
                                if !output.is_empty() {
                                    output.push('\n');
                                }
                                output.push_str(&stderr);
                            }
                            if output.len() > 100_000 {
                                output.truncate(100_000);
                                output.push_str("\n... (output truncated)");
                            }
                            if status.success() {
                                Ok(output)
                            } else {
                                Err(format!("{}\nexit code: {}", output, status.code().unwrap_or(-1)))
                            }
                        }
                        Ok(Err(e)) => Err(format!("failed to run command: {}", e)),
                        Err(_) => {
                            child.kill().await.ok();
                            Err(format!("command timed out after {}ms", timeout_ms))
                        }
                    }
                })
                .await;
            return Ok(ToolResult::text(format!(
                "Background task started: {}. Use TaskOutput to check results.",
                task_id
            )));
        }

        let timeout = std::time::Duration::from_millis(timeout_ms.min(600_000));

        let mut child = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&ctx.working_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let result = tokio::time::timeout(timeout, async {
            let mut stdout = String::new();
            let mut stderr = String::new();

            if let Some(mut out) = child.stdout.take() {
                out.read_to_string(&mut stdout).await.ok();
            }
            if let Some(mut err) = child.stderr.take() {
                err.read_to_string(&mut stderr).await.ok();
            }

            let status = child.wait().await?;
            Ok::<_, anyhow::Error>((stdout, stderr, status))
        })
        .await;

        match result {
            Ok(Ok((stdout, stderr, status))) => {
                let mut output = String::new();
                if !stdout.is_empty() {
                    output.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&stderr);
                }

                // Truncate very long outputs
                if output.len() > 100_000 {
                    output.truncate(100_000);
                    output.push_str("\n... (output truncated)");
                }

                if status.success() {
                    Ok(ToolResult::text(output))
                } else {
                    let code = status.code().unwrap_or(-1);
                    if output.is_empty() {
                        Ok(ToolResult::error(format!("exit code: {}", code)))
                    } else {
                        Ok(ToolResult::error(format!(
                            "{}\nexit code: {}",
                            output, code
                        )))
                    }
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!("failed to run command: {}", e))),
            Err(_) => {
                // Timeout: kill the process
                child.kill().await.ok();
                Ok(ToolResult::error(format!(
                    "command timed out after {}ms",
                    timeout_ms
                )))
            }
        }
    }

    fn format_for_display(&self, input: &Value) -> String {
        let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("?");
        format!("$ {}", cmd)
    }
}
