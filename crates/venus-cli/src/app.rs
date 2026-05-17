use std::sync::Arc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use venus_core::engine::QueryEngine;
use venus_core::message::ContentBlock;
use venus_core::stream::StreamEvent;
use venus_core::skill::SkillRegistry;
use venus_utils::session::{self, SessionMeta};

use crate::event::AppEvent;
use crate::input_state::InputState;
use venus_permissions::tui_handler::{PermissionRequest, PermissionResponse};

/// Braille spinner frames.
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A segment of rendered assistant response.
#[derive(Clone, Debug)]
pub enum RenderSegment {
    Text(String),
}

/// A single conversation message for display.
#[derive(Clone, Debug)]
pub enum DisplayMessage {
    User { text: String },
    Assistant { segments: Vec<RenderSegment> },
    ToolCall { name: String, activity: String, is_error: bool, summary: String },
    Error { text: String },
    Status { text: String },
}

/// Input mode determines how key events are handled.
#[derive(PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Streaming,
    PermissionPrompt,
}

/// A pending permission request shown as a modal.
pub struct PendingPermission {
    pub tool_name: String,
    pub description: String,
    pub response_tx: tokio::sync::oneshot::Sender<PermissionResponse>,
}

/// Spinner animation state.
pub struct SpinnerState {
    pub frame: usize,
    pub message: String,
    pub active: bool,
}

/// Top-level TUI application state.
pub struct App {
    pub engine: QueryEngine,
    pub messages: Vec<DisplayMessage>,
    pub input: InputState,
    pub input_mode: InputMode,
    pub spinner: SpinnerState,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub cost: String,
    pub branch: Option<String>,
    pub should_quit: bool,
    pub tick_count: u64,
    pub skill_registry: Option<Arc<SkillRegistry>>,
    pub plugin_registry: Option<venus_core::plugin_registry::PluginRegistry>,
    /// Channel to send AppEvents (used for forwarding stream events).
    pub event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Pending permission request (shown as modal).
    pub pending_permission: Option<PendingPermission>,
}

impl App {
    pub fn new(
        engine: QueryEngine,
        skill_registry: Option<Arc<SkillRegistry>>,
        plugin_registry: Option<venus_core::plugin_registry::PluginRegistry>,
        event_tx: mpsc::UnboundedSender<AppEvent>,
    ) -> Self {
        let cost = engine.cost_tracker.lock().unwrap().format_cost();
        let branch = get_git_branch(&engine.working_dir);

        Self {
            engine,
            messages: Vec::new(),
            input: InputState::new(),
            input_mode: InputMode::Normal,
            spinner: SpinnerState { frame: 0, message: String::new(), active: false },
            scroll_offset: 0,
            auto_scroll: true,
            cost,
            branch,
            should_quit: false,
            tick_count: 0,
            skill_registry,
            plugin_registry,
            event_tx,
            pending_permission: None,
        }
    }

    /// Handle a mouse event (scroll).
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.auto_scroll = false;
                self.scroll_offset = self.scroll_offset.saturating_add(3);
            }
            MouseEventKind::ScrollDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(3);
                if self.scroll_offset == 0 {
                    self.auto_scroll = true;
                }
            }
            _ => {}
        }
    }

    /// Handle a keyboard event.
    pub async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        // Global shortcuts
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                if self.input_mode == InputMode::Streaming {
                    self.engine.cancel_token.cancel();
                    self.input_mode = InputMode::Normal;
                    self.spinner.active = false;
                    self.messages.push(DisplayMessage::Status {
                        text: "cancelled".to_string(),
                    });
                } else {
                    self.should_quit = true;
                }
                return Ok(());
            }
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                self.should_quit = true;
                return Ok(());
            }
            _ => {}
        }

        // Permission prompt mode
        if self.input_mode == InputMode::PermissionPrompt {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.respond_permission(PermissionResponse::Allow);
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.respond_permission(PermissionResponse::Deny);
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    self.respond_permission(PermissionResponse::AlwaysAllow);
                }
                KeyCode::Char('d') | KeyCode::Char('D') => {
                    self.respond_permission(PermissionResponse::NeverAllow);
                }
                KeyCode::Esc => {
                    self.respond_permission(PermissionResponse::Deny);
                }
                _ => {}
            }
            return Ok(());
        }

        // During streaming, only Ctrl+C is handled
        if self.input_mode == InputMode::Streaming {
            return Ok(());
        }

        // Normal input mode
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Enter) => {
                let input = self.input.take_buffer();
                if !input.trim().is_empty() {
                    self.submit_input(&input).await?;
                }
            }
            (KeyModifiers::ALT, KeyCode::Enter) => {
                self.input.insert_char('\n');
            }
            (KeyModifiers::NONE, KeyCode::Esc) => {
                self.input.clear();
            }
            (KeyModifiers::NONE, KeyCode::Tab) => {
                if self.input.buffer.starts_with('/') {
                    self.input.complete_slash();
                    if !self.input.completion_matches.is_empty() {
                        self.input.accept_completion();
                    }
                }
            }
            (KeyModifiers::NONE, KeyCode::Up) => {
                self.input.history_up();
            }
            (KeyModifiers::NONE, KeyCode::Down) => {
                self.input.history_down();
            }
            (KeyModifiers::NONE, KeyCode::PageUp) => {
                self.auto_scroll = false;
                self.scroll_offset = self.scroll_offset.saturating_add(10);
            }
            (KeyModifiers::NONE, KeyCode::PageDown) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                if self.scroll_offset == 0 {
                    self.auto_scroll = true;
                }
            }
            (KeyModifiers::SHIFT, KeyCode::Tab) => {
                self.cycle_permission_mode();
            }
            (_, KeyCode::Char(c)) => {
                self.input.insert_char(c);
            }
            (KeyModifiers::NONE, KeyCode::Backspace) => {
                self.input.backspace();
            }
            (KeyModifiers::NONE, KeyCode::Delete) => {
                self.input.delete();
            }
            (KeyModifiers::NONE, KeyCode::Left) => {
                self.input.move_cursor_left();
            }
            (KeyModifiers::NONE, KeyCode::Right) => {
                self.input.move_cursor_right();
            }
            (KeyModifiers::NONE, KeyCode::Home)
            | (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                self.input.move_cursor_home();
            }
            (KeyModifiers::NONE, KeyCode::End)
            | (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
                self.input.move_cursor_end();
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle a stream event from the engine.
    pub fn handle_stream_event(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::TextDelta(text) => {
                self.spinner.active = false;
                self.append_text_delta(&text);
            }
            StreamEvent::ThinkingDelta(_) => {
                // Hidden by default
            }
            StreamEvent::ToolUseStart { name, .. } => {
                self.spinner.active = false;
                self.messages.push(DisplayMessage::ToolCall {
                    name,
                    activity: String::new(),
                    is_error: false,
                    summary: String::new(),
                });
            }
            StreamEvent::ToolUseInput(json) => {
                if let Some(DisplayMessage::ToolCall { name, activity, .. }) =
                    self.messages.iter_mut().rev().find(|m| matches!(m, DisplayMessage::ToolCall { activity, .. } if activity.is_empty()))
                {
                    if let Some(act) = tool_activity_from_json(name, &json) {
                        *activity = act;
                    }
                }
            }
            StreamEvent::ToolResult { name, result, .. } => {
                let text: String = result
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let line_count = text.lines().count().max(1);
                let char_count = text.chars().count();
                let summary = format!("{} ({} lines, {} chars)", name, line_count, char_count);

                // Find the last ToolCall for this name and update it
                if let Some(DisplayMessage::ToolCall {
                    name: n, is_error, summary: s, ..
                }) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| matches!(m, DisplayMessage::ToolCall { name: n, .. } if n == &name))
                {
                    *is_error = result.is_error;
                    *s = summary;
                }
            }
            StreamEvent::MessageComplete(_) => {
                self.input_mode = InputMode::Normal;
                self.spinner.active = false;
                self.cost = self.engine.cost_tracker.lock().unwrap().format_cost();
                self.save_session();
            }
            StreamEvent::Error(err) => {
                self.messages.push(DisplayMessage::Error { text: err });
                self.input_mode = InputMode::Normal;
                self.spinner.active = false;
            }
            StreamEvent::Usage(usage) => {
                let total = usage.input_tokens + usage.cache_read_tokens + usage.output_tokens;
                self.messages.push(DisplayMessage::Status {
                    text: format!(
                        "tokens: {} (in:{} out:{})",
                        format_token_count(total),
                        format_token_count(usage.input_tokens + usage.cache_read_tokens),
                        format_token_count(usage.output_tokens),
                    ),
                });
                self.cost = self.engine.cost_tracker.lock().unwrap().format_cost();
            }
            StreamEvent::AutoCompacted {
                messages_removed,
                tokens_saved,
            } => {
                self.messages.push(DisplayMessage::Status {
                    text: format!(
                        "[auto-compacted: removed {} messages, ~{} tokens saved]",
                        messages_removed, tokens_saved
                    ),
                });
            }
        }

        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }

    /// Handle an incoming permission request from the engine.
    pub fn handle_permission_request(&mut self, req: PermissionRequest) {
        self.pending_permission = Some(PendingPermission {
            tool_name: req.tool_name,
            description: req.description,
            response_tx: req.response_tx,
        });
        self.input_mode = InputMode::PermissionPrompt;
    }

    /// Respond to the current permission request.
    fn respond_permission(&mut self, response: PermissionResponse) {
        if let Some(pending) = self.pending_permission.take() {
            let _ = pending.response_tx.send(response);
        }
        // Return to previous mode
        if self.spinner.active {
            self.input_mode = InputMode::Streaming;
        } else {
            self.input_mode = InputMode::Normal;
        }
    }

    /// Get the model name.
    pub fn model(&self) -> &str {
        &self.engine.model
    }

    /// Handle a cron-scheduled prompt.
    pub async fn handle_cron_prompt(&mut self, prompt: &str) -> Result<()> {
        self.messages.push(DisplayMessage::Status {
            text: format!("[cron] Executing scheduled prompt..."),
        });

        let content = vec![ContentBlock::text(prompt)];
        self.engine.cancel_token = CancellationToken::new();
        let rx = self.engine.submit_message(content).await?;
        crate::event::spawn_stream_forwarder(rx, self.event_tx.clone());
        self.input_mode = InputMode::Streaming;
        self.spinner = SpinnerState {
            frame: 0,
            message: "Thinking...".to_string(),
            active: true,
        };
        self.auto_scroll = true;
        self.scroll_offset = 0;

        Ok(())
    }

    /// Tick for animation.
    pub fn tick(&mut self) {
        self.tick_count += 1;
        if self.spinner.active {
            self.spinner.frame = self.tick_count as usize % SPINNER_FRAMES.len();
        }
    }

    /// Get the current spinner glyph.
    pub fn spinner_glyph(&self) -> &str {
        SPINNER_FRAMES[self.spinner.frame % SPINNER_FRAMES.len()]
    }

    /// Submit user input to the engine.
    async fn submit_input(&mut self, input: &str) -> Result<()> {
        // Handle slash commands
        if input.starts_with('/') {
            self.messages.push(DisplayMessage::User {
                text: input.to_string(),
            });
            return self.handle_slash_command(input).await;
        }

        // Add user message to display
        self.messages.push(DisplayMessage::User {
            text: input.to_string(),
        });

        // Submit to engine
        let content = vec![ContentBlock::text(input)];
        self.engine.cancel_token = CancellationToken::new();
        let rx = self.engine.submit_message(content).await?;

        // Forward stream events into the AppEvent channel
        crate::event::spawn_stream_forwarder(rx, self.event_tx.clone());

        self.input_mode = InputMode::Streaming;
        self.spinner = SpinnerState {
            frame: 0,
            message: "Thinking...".to_string(),
            active: true,
        };
        self.auto_scroll = true;
        self.scroll_offset = 0;

        Ok(())
    }

    /// Handle a slash command.
    async fn handle_slash_command(&mut self, input: &str) -> Result<()> {
        // Handle TUI-specific commands first
        let cmd = input.split_whitespace().next().unwrap_or("");
        match cmd {
            "/cost" => {
                let cost = self.engine.cost_tracker.lock().unwrap().format_cost();
                let tracker = self.engine.cost_tracker.lock().unwrap();
                let tokens = tracker.format_tokens();
                self.messages.push(DisplayMessage::Status {
                    text: format!("Cost: {} | Tokens: {}", cost, tokens),
                });
                return Ok(());
            }
            "/clear" => {
                self.engine.messages.lock().await.clear();
                self.messages.clear();
                self.messages.push(DisplayMessage::Status {
                    text: "Conversation cleared.".to_string(),
                });
                return Ok(());
            }
            "/model" => {
                let parts: Vec<&str> = input.split_whitespace().collect();
                if let Some(model) = parts.get(1) {
                    self.engine.model = model.to_string();
                    self.messages.push(DisplayMessage::Status {
                        text: format!("Model changed to: {}", model),
                    });
                } else {
                    self.messages.push(DisplayMessage::Status {
                        text: format!("Current model: {}", self.engine.model),
                    });
                }
                return Ok(());
            }
            "/status" => {
                let cost = self.engine.cost_tracker.lock().unwrap().format_cost();
                let msg_count = self.engine.messages.lock().await.len();
                let session_name = self.engine.session_name.as_deref().unwrap_or("(unnamed)");
                self.messages.push(DisplayMessage::Status {
                    text: format!(
                        "Session: {} | Model: {} | Messages: {} | Cost: {}",
                        session_name, self.engine.model, msg_count, cost
                    ),
                });
                return Ok(());
            }
            "/help" => {
                let help_text = "Available commands:\n\
                    /help - Show this help\n\
                    /clear - Clear conversation\n\
                    /cost - Show token usage and cost\n\
                    /model [name] - Show or change model\n\
                    /status - Show session status\n\
                    /compact - Compact conversation\n\
                    /diff - Show git diff\n\
                    /commit - Generate commit message\n\
                    /review - Review code changes\n\
                    /sessions - List saved sessions\n\
                    /resume [id] - Resume session\n\
                    /export [path] - Export conversation\n\
                    /rewind [n] - Rewind n messages\n\
                    /fast - Toggle fast mode\n\
                    /permissions - Show permission mode\n\
                    /quit - Exit";
                self.messages.push(DisplayMessage::Status {
                    text: help_text.to_string(),
                });
                return Ok(());
            }
            "/quit" | "/exit" | "/q" => {
                self.should_quit = true;
                return Ok(());
            }
            _ => {}
        }

        // For other commands, use the existing handler
        let result = crate::commands::handle_command(
            input,
            &mut self.engine,
            self.skill_registry.as_ref(),
            self.plugin_registry.as_ref(),
        )
        .await;

        match result {
            crate::commands::CommandResult::Exit => {
                self.should_quit = true;
            }
            crate::commands::CommandResult::InjectMessage(msg) => {
                let content = vec![ContentBlock::text(&msg)];
                self.engine.cancel_token = CancellationToken::new();
                let rx = self.engine.submit_message(content).await?;
                crate::event::spawn_stream_forwarder(rx, self.event_tx.clone());
                self.input_mode = InputMode::Streaming;
                self.spinner = SpinnerState {
                    frame: 0,
                    message: "Thinking...".to_string(),
                    active: true,
                };
            }
            crate::commands::CommandResult::ToggleVim => {
                // TODO: vim mode in TUI
            }
            crate::commands::CommandResult::Continue => {}
        }

        self.save_session();
        self.cost = self.engine.cost_tracker.lock().unwrap().format_cost();
        Ok(())
    }

    /// Append text delta to the current assistant message.
    fn append_text_delta(&mut self, text: &str) {
        match self.messages.last_mut() {
            Some(DisplayMessage::Assistant { segments }) => match segments.last_mut() {
                Some(RenderSegment::Text(t)) => t.push_str(text),
                _ => segments.push(RenderSegment::Text(text.to_string())),
            },
            _ => {
                self.messages.push(DisplayMessage::Assistant {
                    segments: vec![RenderSegment::Text(text.to_string())],
                });
            }
        }
    }

    /// Cycle permission mode.
    fn cycle_permission_mode(&mut self) {
        let current = self
            .engine
            .settings
            .permission_mode
            .as_deref()
            .unwrap_or("default");
        let next = match current {
            "default" => "auto",
            "auto" => "bypass",
            _ => "default",
        };
        self.engine.settings = Arc::new({
            let mut s = (*self.engine.settings).clone();
            s.permission_mode = Some(next.to_string());
            s
        });
        self.messages.push(DisplayMessage::Status {
            text: format!("Permission mode: {}", next),
        });
    }

    /// Save the current session to disk.
    fn save_session(&self) {
        let now = chrono::Utc::now().timestamp() as u64;
        // We need to block on the async save since we're in a sync context
        let engine = self.engine.clone();
        tokio::spawn(async move {
            let messages = engine.messages.lock().await;
            let meta = SessionMeta {
                id: engine.session_id.clone(),
                project: engine.working_dir.display().to_string(),
                created_at: engine.created_at,
                updated_at: now,
                message_count: messages.len(),
                model: engine.model.clone(),
                name: engine.session_name.clone(),
            };
            let msg_values: Vec<serde_json::Value> =
                messages.iter().filter_map(|m| serde_json::to_value(m).ok()).collect();
            drop(messages);
            if let Err(e) = session::save_session(&engine.session_id, &meta, &msg_values).await {
                tracing::warn!("failed to save session: {}", e);
            }
        });
    }
}

/// Extract a human-readable activity from tool name + JSON input.
fn tool_activity_from_json(tool_name: &str, json: &str) -> Option<String> {
    let input: serde_json::Value = serde_json::from_str(json).ok()?;
    match tool_name {
        "Bash" | "BashTool" => {
            let cmd = input.get("command")?.as_str()?;
            Some(truncate_str(cmd, 80))
        }
        "Read" | "Write" | "Edit" | "FileReadTool" | "FileWriteTool" | "FileEditTool" => {
            let path = input.get("file_path")?.as_str()?;
            Some(path.to_string())
        }
        "Glob" | "GlobTool" => input.get("pattern")?.as_str().map(String::from),
        "Grep" | "GrepTool" => {
            let pattern = input.get("pattern")?.as_str()?;
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            Some(format!("{} in {}", pattern, path))
        }
        "Agent" | "AgentTool" => input.get("description")?.as_str().map(String::from),
        "WebFetch" | "WebFetchTool" => input.get("url")?.as_str().map(String::from),
        "WebSearch" | "WebSearchTool" => input.get("query")?.as_str().map(String::from),
        _ => None,
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars - 3).collect();
        format!("{}...", truncated)
    }
}

fn format_token_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

fn get_git_branch(working_dir: &std::path::Path) -> Option<String> {
    std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(working_dir)
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        })
}
