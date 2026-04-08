pub mod events;
pub mod matcher;

use std::path::PathBuf;

use anyhow::Result;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tracing::{debug, warn};
use venus_utils::config::{CommandHook, HookConfig, HookEntry};

use events::*;

/// Runs shell command hooks at lifecycle events.
pub struct HookRunner {
    config: Option<HookConfig>,
    session_id: String,
    cwd: PathBuf,
}

struct HookOutput {
    exit_code: i32,
    stdout: String,
    #[allow(dead_code)]
    stderr: String,
}

impl HookRunner {
    pub fn new(config: Option<HookConfig>, session_id: String, cwd: PathBuf) -> Self {
        Self {
            config,
            session_id,
            cwd,
        }
    }

    /// Update the session ID (called after engine creates its real session ID).
    pub fn set_session_id(&mut self, session_id: String) {
        self.session_id = session_id;
    }

    /// Run PreToolUse hooks. If any hook denies, returns a deny response.
    pub async fn run_pre_tool_use(
        &self,
        tool_name: &str,
        tool_input: &Value,
    ) -> Result<PreToolUseResponse> {
        let entries = self.get_entries("PreToolUse");
        if entries.is_empty() {
            return Ok(PreToolUseResponse::default());
        }

        let event = HookEvent::PreToolUse {
            session_id: self.session_id.clone(),
            tool_name: tool_name.to_string(),
            tool_input: tool_input.clone(),
        };

        let matched = matcher::matching_hooks(&entries, &event);

        for entry in matched {
            for hook in &entry.hooks {
                match self.execute_command_hook(hook, &event).await {
                    Ok(output) => {
                        if output.exit_code == 2 {
                            return Ok(PreToolUseResponse {
                                decision: Some("deny".into()),
                                reason: if output.stdout.trim().is_empty() {
                                    Some("blocked by hook (exit code 2)".into())
                                } else {
                                    Some(output.stdout.trim().to_string())
                                },
                                updated_input: None,
                            });
                        }

                        if !output.stdout.trim().is_empty() {
                            if let Ok(resp) =
                                serde_json::from_str::<PreToolUseResponse>(output.stdout.trim())
                            {
                                if resp.decision.as_deref() == Some("deny") {
                                    return Ok(resp);
                                }
                                if resp.updated_input.is_some() {
                                    return Ok(resp);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("PreToolUse hook failed: {}", e);
                    }
                }
            }
        }

        Ok(PreToolUseResponse::default())
    }

    /// Run PostToolUse hooks (non-blocking notification).
    pub async fn run_post_tool_use(
        &self,
        tool_name: &str,
        tool_input: &Value,
        result: &str,
        is_error: bool,
    ) {
        let entries = self.get_entries("PostToolUse");
        if entries.is_empty() {
            return;
        }

        let event = HookEvent::PostToolUse {
            session_id: self.session_id.clone(),
            tool_name: tool_name.to_string(),
            tool_input: tool_input.clone(),
            tool_result: result.to_string(),
            is_error,
        };

        let matched = matcher::matching_hooks(&entries, &event);
        for entry in matched {
            for hook in &entry.hooks {
                if let Err(e) = self.execute_command_hook(hook, &event).await {
                    warn!("PostToolUse hook failed: {}", e);
                }
            }
        }
    }

    /// Run UserPromptSubmit hooks. Can modify or block the prompt.
    pub async fn run_user_prompt_submit(&self, prompt: &str) -> Result<UserPromptSubmitResponse> {
        let entries = self.get_entries("UserPromptSubmit");
        if entries.is_empty() {
            return Ok(UserPromptSubmitResponse::default());
        }

        let event = HookEvent::UserPromptSubmit {
            session_id: self.session_id.clone(),
            prompt: prompt.to_string(),
        };

        let matched = matcher::matching_hooks(&entries, &event);
        for entry in matched {
            for hook in &entry.hooks {
                match self.execute_command_hook(hook, &event).await {
                    Ok(output) => {
                        if output.exit_code == 2 {
                            return Ok(UserPromptSubmitResponse {
                                updated_prompt: None,
                                deny: Some(true),
                                reason: if output.stdout.trim().is_empty() {
                                    Some("blocked by hook".into())
                                } else {
                                    Some(output.stdout.trim().to_string())
                                },
                            });
                        }

                        if !output.stdout.trim().is_empty() {
                            if let Ok(resp) = serde_json::from_str::<UserPromptSubmitResponse>(
                                output.stdout.trim(),
                            ) {
                                if resp.deny == Some(true) || resp.updated_prompt.is_some() {
                                    return Ok(resp);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("UserPromptSubmit hook failed: {}", e);
                    }
                }
            }
        }

        Ok(UserPromptSubmitResponse::default())
    }

    /// Run simple notification hooks (SessionStart, PreCompact, PostCompact, Stop).
    pub async fn run_simple_event(&self, event: HookEvent) {
        let entries = self.get_entries(event.event_name());
        if entries.is_empty() {
            return;
        }

        let matched = matcher::matching_hooks(&entries, &event);
        for entry in matched {
            for hook in &entry.hooks {
                if let Err(e) = self.execute_command_hook(hook, &event).await {
                    warn!("{} hook failed: {}", event.event_name(), e);
                }
            }
        }
    }

    /// Execute a single command hook: spawn subprocess, pipe JSON to stdin, read stdout.
    async fn execute_command_hook(
        &self,
        hook: &CommandHook,
        event: &HookEvent,
    ) -> Result<HookOutput> {
        let payload = serde_json::to_string(event)?;

        debug!(
            "executing hook: {} for event {}",
            hook.command,
            event.event_name()
        );

        let mut child = tokio::process::Command::new("sh")
            .args(["-c", &hook.command])
            .current_dir(&self.cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        // Write JSON payload to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(payload.as_bytes()).await?;
            // Drop stdin to close the pipe and signal EOF
        }

        // Wait with timeout (timeout is in seconds)
        let timeout_secs = if hook.timeout > 0 { hook.timeout } else { 10 };
        let timeout = tokio::time::Duration::from_secs(timeout_secs);
        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| anyhow::anyhow!("hook timed out after {}s", timeout_secs))??;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        debug!(
            "hook completed: exit_code={}, stdout_len={}, stderr_len={}",
            exit_code,
            stdout.len(),
            stderr.len()
        );

        Ok(HookOutput {
            exit_code,
            stdout,
            stderr,
        })
    }

    /// Get hook entries for the given event name from config.
    fn get_entries(&self, event_name: &str) -> Vec<HookEntry> {
        self.config
            .as_ref()
            .and_then(|c| c.entries.get(event_name))
            .cloned()
            .unwrap_or_default()
    }
}
