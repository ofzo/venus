use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::process::Command;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct EnterWorktreeTool;

#[async_trait]
impl Tool for EnterWorktreeTool {
    fn name(&self) -> &str {
        "EnterWorktree"
    }

    fn description(&self) -> &str {
        "Create an isolated git worktree for development. Changes made in the worktree do not affect the main working directory."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Optional name for the worktree. If not provided, a random name is generated."
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                format!("wt-{}", &uuid::Uuid::new_v4().to_string()[..8])
            });

        // Validate name format: letters, digits, dots, underscores, dashes
        if name.len() > 64
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Ok(ToolResult::error(
                "Invalid worktree name. Use only letters, digits, dots, underscores, and dashes (max 64 chars).",
            ));
        }

        // Check if we're in a git repo
        if !venus_utils::git::is_git_repo(&ctx.working_dir).await {
            return Ok(ToolResult::error("Not in a git repository."));
        }

        // Check if already in a worktree
        let common_dir_output = Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(&ctx.working_dir)
            .output()
            .await?;
        let git_dir_output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(&ctx.working_dir)
            .output()
            .await?;

        let common_dir = String::from_utf8_lossy(&common_dir_output.stdout)
            .trim()
            .to_string();
        let git_dir = String::from_utf8_lossy(&git_dir_output.stdout)
            .trim()
            .to_string();

        if common_dir != git_dir {
            return Ok(ToolResult::error(
                "Already in a worktree. Exit the current worktree first.",
            ));
        }

        // Create worktree directory under .venus/worktrees/
        let worktree_base = ctx.working_dir.join(".venus").join("worktrees");
        tokio::fs::create_dir_all(&worktree_base).await?;

        let worktree_path = worktree_base.join(&name);
        let branch_name = format!("venus/{}", name);

        // Create git worktree with new branch
        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                &branch_name,
                worktree_path.to_str().unwrap_or_default(),
            ])
            .current_dir(&ctx.working_dir)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Ok(ToolResult::error(format!(
                "Failed to create worktree: {}",
                stderr.trim()
            )));
        }

        Ok(ToolResult::text(format!(
            "Created worktree at: {}\nBranch: {}\n\
             The session's working directory has been switched to the worktree.",
            worktree_path.display(),
            branch_name,
        )))
    }
}

pub struct ExitWorktreeTool;

#[async_trait]
impl Tool for ExitWorktreeTool {
    fn name(&self) -> &str {
        "ExitWorktree"
    }

    fn description(&self) -> &str {
        "Exit the current worktree session. Choose to keep the worktree on disk or remove it."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["keep", "remove"],
                    "description": "Whether to keep or remove the worktree"
                },
                "discard_changes": {
                    "type": "boolean",
                    "description": "Force remove even with uncommitted changes",
                    "default": false
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'action' parameter"))?;

        let discard = input
            .get("discard_changes")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Check if we're in a worktree
        let common_dir_output = Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(&ctx.working_dir)
            .output()
            .await?;
        let git_dir_output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(&ctx.working_dir)
            .output()
            .await?;

        let common_dir = String::from_utf8_lossy(&common_dir_output.stdout)
            .trim()
            .to_string();
        let git_dir = String::from_utf8_lossy(&git_dir_output.stdout)
            .trim()
            .to_string();

        if common_dir == git_dir {
            return Ok(ToolResult::error("Not currently in a worktree."));
        }

        let worktree_path = ctx.working_dir.to_string_lossy().to_string();

        match action {
            "keep" => Ok(ToolResult::text(format!(
                "Worktree kept at: {}\nSession returned to original directory.",
                worktree_path
            ))),
            "remove" => {
                if !discard {
                    // Check for uncommitted changes
                    let status = Command::new("git")
                        .args(["status", "--porcelain"])
                        .current_dir(&ctx.working_dir)
                        .output()
                        .await?;
                    let status_text =
                        String::from_utf8_lossy(&status.stdout).trim().to_string();
                    if !status_text.is_empty() {
                        return Ok(ToolResult::error(format!(
                            "Worktree has uncommitted changes:\n{}\n\
                             Use discard_changes: true to force remove.",
                            status_text
                        )));
                    }
                }

                // Resolve the main repo directory from the common git dir
                let main_dir = std::path::Path::new(&common_dir)
                    .parent()
                    .unwrap_or(std::path::Path::new(&common_dir));

                let mut args = vec!["worktree", "remove"];
                if discard {
                    args.push("--force");
                }
                args.push(&worktree_path);

                let output = Command::new("git")
                    .args(&args)
                    .current_dir(main_dir)
                    .output()
                    .await?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Ok(ToolResult::error(format!(
                        "Failed to remove worktree: {}",
                        stderr.trim()
                    )));
                }

                Ok(ToolResult::text(
                    "Worktree removed. Session returned to original directory.",
                ))
            }
            _ => Ok(ToolResult::error(
                "action must be 'keep' or 'remove'",
            )),
        }
    }
}
