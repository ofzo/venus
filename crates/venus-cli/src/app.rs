use std::sync::Arc;
use std::time::Instant;

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

/// Spinner frames matching Claude Code's Unicode characters.
const SPINNER_FRAMES: &[&str] = &["·", "✂", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✂"];

/// Random verbs shown during spinner (matching Claude Code's behavior).
const SPINNER_VERBS: &[&str] = &[
    "Thinking", "Processing", "Analyzing", "Computing", "Working",
    "Reasoning", "Planning", "Searching", "Reading", "Writing",
];

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

/// A single item in a picker list.
#[derive(Clone, Debug)]
pub struct PickerItem {
    pub label: String,
    pub description: String,
    pub value: String,
}

/// Which picker is currently active.
#[derive(Clone, Debug)]
pub enum PickerSource {
    Model,
    Theme,
    Help,
    Resume(Vec<venus_utils::session::SessionMeta>),
    Permissions,
    Effort,
    Skills(Vec<String>),
    Config,
    ConfigSub(String), // Config sub-picker for a specific setting
}

/// State for an active picker/list selection overlay.
pub struct PickerState {
    pub title: String,
    pub items: Vec<PickerItem>,
    pub selected: usize,
    pub scroll_offset: usize,
    pub source: PickerSource,
    pub visible_count: usize,
}

impl PickerState {
    pub fn new(title: String, items: Vec<PickerItem>, source: PickerSource) -> Self {
        let visible_count = items.len().min(10);
        Self { title, items, selected: 0, scroll_offset: 0, source, visible_count }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.selected < self.scroll_offset {
                self.scroll_offset = self.selected;
            }
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
            if self.selected >= self.scroll_offset + self.visible_count {
                self.scroll_offset = self.selected - self.visible_count + 1;
            }
        }
    }

    pub fn selected_item(&self) -> Option<&PickerItem> {
        self.items.get(self.selected)
    }
}

/// Input mode determines how key events are handled.
#[derive(PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Streaming,
    PermissionPrompt,
    Picker,
    HistorySearch,
}

/// A pending permission request shown as a modal.
pub struct PendingPermission {
    pub tool_name: String,
    pub description: String,
    pub response_tx: tokio::sync::oneshot::Sender<PermissionResponse>,
    pub selected_option: usize,
}

/// Spinner animation state.
pub struct SpinnerState {
    pub frame: usize,
    pub message: String,
    pub active: bool,
    pub started_at: Option<Instant>,
    pub verb_index: usize,
}

impl SpinnerState {
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }

    /// Get the display message with verb and elapsed time (after 30s).
    pub fn display_message(&self) -> String {
        let verb = SPINNER_VERBS[self.verb_index % SPINNER_VERBS.len()];
        let elapsed = self.elapsed_secs();
        if elapsed >= 30 {
            format!("{}... ({}s)", verb, elapsed)
        } else {
            format!("{}...", verb)
        }
    }
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
    /// Active picker/list selection overlay.
    pub picker: Option<PickerState>,
    /// Double-press tracking for Ctrl+C (timestamp of first press).
    pub ctrl_c_first: Option<Instant>,
    /// Double-press tracking for Esc (timestamp of first press).
    pub esc_first: Option<Instant>,
    /// Context window usage percentage.
    pub context_pct: u64,
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
            spinner: SpinnerState { frame: 0, message: String::new(), active: false, started_at: None, verb_index: 0 },
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
            picker: None,
            ctrl_c_first: None,
            esc_first: None,
            context_pct: 0,
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
        // Global shortcuts (work in all modes)
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                if self.input_mode == InputMode::Streaming {
                    self.engine.cancel_token.cancel();
                    self.input_mode = InputMode::Normal;
                    self.spinner.active = false;
                    self.messages.push(DisplayMessage::Status {
                        text: "cancelled".to_string(),
                    });
                    return Ok(());
                } else if self.input_mode == InputMode::Picker {
                    self.picker = None;
                    self.input_mode = InputMode::Normal;
                    return Ok(());
                } else if self.input_mode == InputMode::HistorySearch {
                    self.input_mode = InputMode::Normal;
                    self.input.clear();
                    return Ok(());
                } else {
                    // Double-press: first press warns, second press exits
                    let now = Instant::now();
                    if let Some(first) = self.ctrl_c_first {
                        if now.duration_since(first).as_millis() < 800 {
                            self.should_quit = true;
                            return Ok(());
                        }
                    }
                    self.ctrl_c_first = Some(now);
                    if !self.input.buffer.is_empty() {
                        self.input.clear();
                    } else {
                        self.messages.push(DisplayMessage::Status {
                            text: "Press Ctrl+C again to exit".to_string(),
                        });
                    }
                    return Ok(());
                }
            }
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                self.should_quit = true;
                return Ok(());
            }
            (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
                // Redraw - ratatui handles this on next frame, just reset scroll
                self.auto_scroll = true;
                self.scroll_offset = 0;
                return Ok(());
            }
            (KeyModifiers::CONTROL, KeyCode::Char('r')) => {
                // History search - activate if we have history
                if !self.input.history.is_empty() && self.input_mode == InputMode::Normal {
                    self.open_history_search();
                }
                return Ok(());
            }
            // Meta+P (Alt+P) - model picker
            (KeyModifiers::ALT, KeyCode::Char('p')) => {
                if self.input_mode == InputMode::Normal {
                    self.open_model_picker();
                }
                return Ok(());
            }
            // Meta+T (Alt+T) - thinking toggle
            (KeyModifiers::ALT, KeyCode::Char('t')) => {
                if self.input_mode == InputMode::Normal {
                    self.toggle_thinking();
                }
                return Ok(());
            }
            // Meta+O (Alt+O) - fast mode toggle
            (KeyModifiers::ALT, KeyCode::Char('o')) => {
                if self.input_mode == InputMode::Normal {
                    self.toggle_fast_mode();
                }
                return Ok(());
            }
            // Ctrl+Shift+P - quick open command palette
            (KeyModifiers::CONTROL | KeyModifiers::SHIFT, KeyCode::Char('p')) => {
                if self.input_mode == InputMode::Normal {
                    self.open_help_picker();
                }
                return Ok(());
            }
            // Ctrl+Shift+F - search messages (future)
            (KeyModifiers::CONTROL | KeyModifiers::SHIFT, KeyCode::Char('f')) => {
                if self.input_mode == InputMode::Normal {
                    self.open_history_search();
                }
                return Ok(());
            }
            // Ctrl+G - open external editor
            (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
                if self.input_mode == InputMode::Normal {
                    self.open_external_editor().await;
                }
                return Ok(());
            }
            // Ctrl+S - stash current prompt (save to history without submitting)
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                if self.input_mode == InputMode::Normal && !self.input.buffer.is_empty() {
                    self.input.history.push(self.input.buffer.clone());
                    self.messages.push(DisplayMessage::Status {
                        text: "Prompt stashed (use Up arrow to recall)".to_string(),
                    });
                    self.input.clear();
                }
                return Ok(());
            }
            _ => {}
        }

        // Picker mode navigation
        if self.input_mode == InputMode::Picker {
            match (key.modifiers, key.code) {
                (_, KeyCode::Esc) => {
                    self.picker = None;
                    self.input_mode = InputMode::Normal;
                }
                (_, KeyCode::Up) | (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
                    if let Some(ref mut p) = self.picker {
                        p.move_up();
                    }
                }
                (_, KeyCode::Down) | (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
                    if let Some(ref mut p) = self.picker {
                        p.move_down();
                    }
                }
                // Left/Right arrows - adjust effort in model picker
                (_, KeyCode::Left) => {
                    if let Some(ref p) = self.picker {
                        if matches!(p.source, PickerSource::Model) {
                            self.cycle_effort(-1);
                        }
                    }
                }
                (_, KeyCode::Right) => {
                    if let Some(ref p) = self.picker {
                        if matches!(p.source, PickerSource::Model) {
                            self.cycle_effort(1);
                        }
                    }
                }
                (_, KeyCode::Enter) => {
                    self.handle_picker_select().await?;
                }
                _ => {}
            }
            return Ok(());
        }

        // History search mode
        if self.input_mode == InputMode::HistorySearch {
            match (key.modifiers, key.code) {
                (_, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                    self.input_mode = InputMode::Normal;
                    self.input.clear();
                }
                (_, KeyCode::Enter) => {
                    // Accept the current history match and submit
                    self.input_mode = InputMode::Normal;
                    let input = self.input.take_buffer();
                    if !input.trim().is_empty() {
                        self.submit_input(&input).await?;
                    }
                }
                (_, KeyCode::Char(c)) => {
                    self.input.insert_char(c);
                    self.history_search_update();
                }
                (KeyModifiers::NONE, KeyCode::Backspace) => {
                    self.input.backspace();
                    self.history_search_update();
                }
                _ => {}
            }
            return Ok(());
        }

        // Permission prompt mode (Select-style navigation)
        if self.input_mode == InputMode::PermissionPrompt {
            match (key.modifiers, key.code) {
                // Up/Down to navigate options
                (_, KeyCode::Up) | (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
                    if let Some(ref mut pending) = self.pending_permission {
                        if pending.selected_option > 0 {
                            pending.selected_option -= 1;
                        }
                    }
                }
                (_, KeyCode::Down) | (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
                    if let Some(ref mut pending) = self.pending_permission {
                        if pending.selected_option < 2 {
                            pending.selected_option += 1;
                        }
                    }
                }
                // Enter to select current option
                (_, KeyCode::Enter) => {
                    self.select_permission_option();
                }
                // Esc to cancel (deny)
                (_, KeyCode::Esc) => {
                    self.respond_permission(PermissionResponse::Deny);
                }
                // Direct key shortcuts (still supported for quick access)
                (_, KeyCode::Char('y') | KeyCode::Char('Y')) => {
                    self.respond_permission(PermissionResponse::Allow);
                }
                (_, KeyCode::Char('n') | KeyCode::Char('N')) => {
                    self.respond_permission(PermissionResponse::Deny);
                }
                (_, KeyCode::Char('a') | KeyCode::Char('A')) => {
                    self.respond_permission(PermissionResponse::AlwaysAllow);
                }
                _ => {}
            }
            return Ok(());
        }

        // During streaming, only Ctrl+C is handled (already done above)
        if self.input_mode == InputMode::Streaming {
            return Ok(());
        }

        // Normal input mode
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Enter) => {
                self.ctrl_c_first = None;
                self.esc_first = None;
                let input = self.input.take_buffer();
                if !input.trim().is_empty() {
                    self.submit_input(&input).await?;
                }
            }
            (KeyModifiers::ALT, KeyCode::Enter) => {
                self.input.insert_char('\n');
            }
            (KeyModifiers::NONE, KeyCode::Esc) => {
                // Double-press Esc: first clears completions/history, second clears input
                if !self.input.completion_matches.is_empty() {
                    self.input.clear_completions();
                    return Ok(());
                }
                if self.input.history_index.is_some() {
                    self.input.history_index = None;
                    self.input.buffer = self.input.history_working.clone();
                    self.input.cursor_pos = self.input.buffer.len();
                    return Ok(());
                }
                let now = Instant::now();
                if let Some(first) = self.esc_first {
                    if now.duration_since(first).as_millis() < 800 {
                        self.input.clear();
                        self.esc_first = None;
                        return Ok(());
                    }
                }
                self.esc_first = Some(now);
                if !self.input.buffer.is_empty() {
                    self.messages.push(DisplayMessage::Status {
                        text: "Press Esc again to clear input".to_string(),
                    });
                }
            }
            (KeyModifiers::NONE, KeyCode::Tab) => {
                if self.input.buffer.starts_with('/') {
                    self.input.complete_slash();
                    if !self.input.completion_matches.is_empty() {
                        self.input.accept_completion();
                    }
                } else if self.input.file_completion_active {
                    self.input.complete_file_path(&self.engine.working_dir);
                    if !self.input.file_completions.is_empty() {
                        self.input.accept_file_completion();
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
            // Ctrl+Home - scroll to top
            (KeyModifiers::CONTROL, KeyCode::Home) => {
                self.auto_scroll = false;
                self.scroll_offset = u16::MAX;
            }
            // Ctrl+End - scroll to bottom
            (KeyModifiers::CONTROL, KeyCode::End) => {
                self.auto_scroll = true;
                self.scroll_offset = 0;
            }
            (KeyModifiers::SHIFT, KeyCode::Tab) => {
                self.cycle_permission_mode();
            }
            // Ctrl+A - cursor to start
            (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                self.input.move_cursor_home();
            }
            // Ctrl+E - cursor to end
            (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
                self.input.move_cursor_end();
            }
            // Ctrl+K - delete to end of line
            (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
                self.input.delete_to_end();
            }
            // Ctrl+U - delete to start of line
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.input.delete_to_start();
            }
            // Ctrl+W - delete word backward
            (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
                self.input.delete_word_backward();
            }
            (_, KeyCode::Char(c)) => {
                self.ctrl_c_first = None;
                self.esc_first = None;
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
            (KeyModifiers::NONE, KeyCode::Home) => {
                self.input.move_cursor_home();
            }
            (KeyModifiers::NONE, KeyCode::End) => {
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
                self.update_context_pct();
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
            selected_option: 0,
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

    /// Select the current permission option.
    fn select_permission_option(&mut self) {
        if let Some(ref pending) = self.pending_permission {
            let response = match pending.selected_option {
                0 => PermissionResponse::Allow,
                1 => PermissionResponse::AlwaysAllow,
                2 => PermissionResponse::Deny,
                _ => PermissionResponse::Deny,
            };
            self.respond_permission(response);
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
            started_at: Some(Instant::now()),
            verb_index: self.tick_count as usize % SPINNER_VERBS.len(),
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
        // Clear double-press timeouts
        if let Some(first) = self.ctrl_c_first {
            if first.elapsed().as_millis() > 800 {
                self.ctrl_c_first = None;
            }
        }
        if let Some(first) = self.esc_first {
            if first.elapsed().as_millis() > 800 {
                self.esc_first = None;
            }
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
            started_at: Some(Instant::now()),
            verb_index: self.tick_count as usize % SPINNER_VERBS.len(),
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
                    self.open_model_picker();
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
                self.open_help_picker();
                return Ok(());
            }
            "/quit" | "/exit" | "/q" => {
                self.should_quit = true;
                return Ok(());
            }
            "/resume" | "/continue" => {
                let parts: Vec<&str> = input.split_whitespace().collect();
                if parts.get(1).is_some() {
                    // Has argument - delegate to command handler
                } else {
                    // No argument - open session picker
                    self.open_resume_picker().await;
                    return Ok(());
                }
            }
            "/theme" => {
                let parts: Vec<&str> = input.split_whitespace().collect();
                if parts.get(1).is_some() {
                    // Has argument - delegate to command handler
                } else {
                    self.open_theme_picker();
                    return Ok(());
                }
            }
            "/permissions" | "/allowed-tools" => {
                let parts: Vec<&str> = input.split_whitespace().collect();
                if parts.get(1).is_some() {
                    // Has argument - delegate to command handler
                } else {
                    self.open_permissions_picker();
                    return Ok(());
                }
            }
            "/fast" => {
                self.toggle_fast_mode();
                return Ok(());
            }
            "/effort" => {
                let parts: Vec<&str> = input.split_whitespace().collect();
                if parts.get(1).is_none() {
                    self.open_effort_picker();
                    return Ok(());
                }
            }
            "/skills" => {
                self.open_skills_picker();
                return Ok(());
            }
            "/config" | "/settings" => {
                let parts: Vec<&str> = input.split_whitespace().collect();
                if parts.get(1).is_none() {
                    self.open_config_picker();
                    return Ok(());
                }
            }
            "/diff" => {
                self.open_diff_viewer().await;
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
                    started_at: Some(Instant::now()),
                    verb_index: self.tick_count as usize % SPINNER_VERBS.len(),
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

    /// Open the model picker overlay matching Claude Code's ModelPicker.
    fn open_model_picker(&mut self) {
        let current_effort = self.get_effort_label();
        let models = vec![
            PickerItem {
                label: "claude-opus-4-20250514".into(),
                description: "Most capable, highest cost".into(),
                value: "claude-opus-4-20250514".into(),
            },
            PickerItem {
                label: "claude-sonnet-4-20250514".into(),
                description: "Balanced capability and cost".into(),
                value: "claude-sonnet-4-20250514".into(),
            },
            PickerItem {
                label: "claude-haiku-4-5-20251001".into(),
                description: "Fastest, lowest cost".into(),
                value: "claude-haiku-4-5-20251001".into(),
            },
        ];
        let current = &self.engine.model;
        let mut picker = PickerState::new("Select model".into(), models, PickerSource::Model);
        if let Some(idx) = picker.items.iter().position(|i| &i.value == current) {
            picker.selected = idx;
            picker.scroll_offset = idx.saturating_sub(picker.visible_count / 2);
        }
        // Store effort info for display
        picker.visible_count = 10; // Claude Code shows up to 10 items
        self.picker = Some(picker);
        self.input_mode = InputMode::Picker;
    }

    /// Get current effort level label.
    pub fn get_effort_label(&self) -> &str {
        match self.engine.settings.thinking.as_ref().and_then(|t| t.budget_tokens) {
            Some(0..=1024) => "low",
            Some(1025..=4096) => "medium",
            Some(4097..=10000) => "high",
            Some(_) => "max",
            None => "medium",
        }
    }

    /// Cycle effort level in the model picker.
    fn cycle_effort(&mut self, direction: i32) {
        let current_budget = self.engine.settings.thinking.as_ref().and_then(|t| t.budget_tokens);
        let current_label = match current_budget {
            Some(0..=1024) => "low",
            Some(1025..=4096) => "medium",
            Some(4097..=10000) => "high",
            Some(_) => "max",
            None => "medium",
        };
        let next = match (current_label, direction) {
            ("low", 1) => "medium",
            ("medium", 1) => "high",
            ("high", 1) => "max",
            ("max", 1) => "low",
            ("low", -1) => "max",
            ("medium", -1) => "low",
            ("high", -1) => "medium",
            ("max", -1) => "high",
            _ => current_label,
        };
        use venus_utils::config::ThinkingConfig;
        let budget = match next {
            "low" => Some(1024),
            "medium" => Some(4096),
            "high" => Some(10000),
            "max" => Some(32000),
            _ => None,
        };
        let existing_mode = self.engine.settings.thinking.as_ref().and_then(|t| t.mode.clone());
        self.engine.settings = Arc::new({
            let mut s = (*self.engine.settings).clone();
            s.thinking = Some(ThinkingConfig {
                mode: existing_mode.or_else(|| Some("enabled".to_string())),
                budget_tokens: budget,
            });
            s
        });
        // Update picker descriptions
        if let Some(ref mut picker) = self.picker {
            if matches!(picker.source, PickerSource::Model) {
                for item in &mut picker.items {
                    let base = item.description.split(" | effort:").next().unwrap_or("");
                    item.description = format!("{} | effort: {}", base, next);
                }
            }
        }
    }

    /// Open the session resume picker.
    async fn open_resume_picker(&mut self) {
        match venus_utils::session::list_sessions().await {
            Ok(sessions) if sessions.is_empty() => {
                self.messages.push(DisplayMessage::Status {
                    text: "No saved sessions.".to_string(),
                });
            }
            Ok(sessions) => {
                let items: Vec<PickerItem> = sessions.iter().map(|s| {
                    let time = chrono::DateTime::from_timestamp(s.updated_at as i64, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    let name = s.name.as_deref().unwrap_or(&s.id[..8.min(s.id.len())]);
                    PickerItem {
                        label: name.to_string(),
                        description: format!("{} msgs | {} | {}", s.message_count, s.model, time),
                        value: s.id.clone(),
                    }
                }).collect();
                let picker = PickerState::new("Resume Session".into(), items, PickerSource::Resume(sessions));
                self.picker = Some(picker);
                self.input_mode = InputMode::Picker;
            }
            Err(e) => {
                self.messages.push(DisplayMessage::Error {
                    text: format!("Failed to list sessions: {}", e),
                });
            }
        }
    }

    /// Open the theme picker.
    fn open_theme_picker(&mut self) {
        let themes = vec![
            PickerItem { label: "dark".into(), description: "Dark theme (default)".into(), value: "dark".into() },
            PickerItem { label: "light".into(), description: "Light theme".into(), value: "light".into() },
            PickerItem { label: "auto".into(), description: "Auto-detect from terminal".into(), value: "auto".into() },
        ];
        let current = &self.engine.theme;
        let mut picker = PickerState::new("Select Theme".into(), themes, PickerSource::Theme);
        if let Some(idx) = picker.items.iter().position(|i| &i.value == current) {
            picker.selected = idx;
        }
        self.picker = Some(picker);
        self.input_mode = InputMode::Picker;
    }

    /// Open the permissions mode picker.
    fn open_permissions_picker(&mut self) {
        let modes = vec![
            PickerItem { label: "default".into(), description: "Ask for permission on risky operations".into(), value: "default".into() },
            PickerItem { label: "auto".into(), description: "Auto-approve most operations".into(), value: "auto".into() },
            PickerItem { label: "bypass".into(), description: "Skip all permission checks (dangerous)".into(), value: "bypass".into() },
        ];
        let current = self.engine.settings.permission_mode.as_deref().unwrap_or("default");
        let mut picker = PickerState::new("Permission Mode".into(), modes, PickerSource::Permissions);
        if let Some(idx) = picker.items.iter().position(|i| i.value == current) {
            picker.selected = idx;
        }
        self.picker = Some(picker);
        self.input_mode = InputMode::Picker;
    }

    /// Open the effort level picker.
    fn open_effort_picker(&mut self) {
        let current_effort = self.engine.settings.thinking.as_ref()
            .and_then(|t| t.budget_tokens)
            .map(|b| match b {
                0..=1024 => "low",
                1025..=4096 => "medium",
                4097..=10000 => "high",
                _ => "max",
            })
            .unwrap_or("medium");

        let items = vec![
            PickerItem { label: "low".into(), description: "Minimal thinking, fastest response".into(), value: "low".into() },
            PickerItem { label: "medium".into(), description: "Balanced thinking".into(), value: "medium".into() },
            PickerItem { label: "high".into(), description: "More thorough thinking".into(), value: "high".into() },
            PickerItem { label: "max".into(), description: "Maximum thinking depth".into(), value: "max".into() },
        ];
        let mut picker = PickerState::new("Effort Level".into(), items, PickerSource::Effort);
        if let Some(idx) = picker.items.iter().position(|i| i.value == current_effort) {
            picker.selected = idx;
        }
        self.picker = Some(picker);
        self.input_mode = InputMode::Picker;
    }

    /// Open the config settings picker.
    fn open_config_picker(&mut self) {
        let perm_mode = self.engine.settings.permission_mode.as_deref().unwrap_or("default");
        let thinking_mode = self.engine.settings.thinking.as_ref()
            .and_then(|t| t.mode.as_deref())
            .unwrap_or("disabled");
        let effort = self.get_effort_label();

        let items = vec![
            PickerItem {
                label: "Model".into(),
                description: format!("Current: {}", self.engine.model),
                value: "model".into(),
            },
            PickerItem {
                label: "Theme".into(),
                description: format!("Current: {}", self.engine.theme),
                value: "theme".into(),
            },
            PickerItem {
                label: "Permission Mode".into(),
                description: format!("Current: {} (Shift+Tab to cycle)", perm_mode),
                value: "permissions".into(),
            },
            PickerItem {
                label: "Thinking Mode".into(),
                description: format!("Current: {} (Alt+T to toggle)", thinking_mode),
                value: "thinking".into(),
            },
            PickerItem {
                label: "Effort Level".into(),
                description: format!("Current: {}", effort),
                value: "effort".into(),
            },
            PickerItem {
                label: "Prompt Color".into(),
                description: format!("Current: {}", self.engine.prompt_color),
                value: "color".into(),
            },
        ];

        let picker = PickerState::new("Settings".into(), items, PickerSource::Config);
        self.picker = Some(picker);
        self.input_mode = InputMode::Picker;
    }

    /// Open a config sub-picker for a specific setting.
    fn open_config_sub_picker(&mut self, setting: &str) {
        let (title, items, source) = match setting {
            "model" => {
                let models = vec![
                    PickerItem { label: "claude-opus-4-20250514".into(), description: "Most capable".into(), value: "claude-opus-4-20250514".into() },
                    PickerItem { label: "claude-sonnet-4-20250514".into(), description: "Balanced".into(), value: "claude-sonnet-4-20250514".into() },
                    PickerItem { label: "claude-haiku-4-5-20251001".into(), description: "Fastest".into(), value: "claude-haiku-4-5-20251001".into() },
                ];
                ("Select Model".into(), models, PickerSource::Model)
            }
            "theme" => {
                let themes = vec![
                    PickerItem { label: "dark".into(), description: "Dark theme".into(), value: "dark".into() },
                    PickerItem { label: "light".into(), description: "Light theme".into(), value: "light".into() },
                    PickerItem { label: "auto".into(), description: "Auto-detect".into(), value: "auto".into() },
                ];
                ("Select Theme".into(), themes, PickerSource::Theme)
            }
            "permissions" => {
                let modes = vec![
                    PickerItem { label: "default".into(), description: "Ask for risky ops".into(), value: "default".into() },
                    PickerItem { label: "auto".into(), description: "Auto-approve most".into(), value: "auto".into() },
                    PickerItem { label: "bypass".into(), description: "Skip all checks".into(), value: "bypass".into() },
                ];
                ("Permission Mode".into(), modes, PickerSource::Permissions)
            }
            "thinking" => {
                let modes = vec![
                    PickerItem { label: "disabled".into(), description: "No thinking".into(), value: "disabled".into() },
                    PickerItem { label: "enabled".into(), description: "Always think".into(), value: "enabled".into() },
                    PickerItem { label: "adaptive".into(), description: "Think when needed".into(), value: "adaptive".into() },
                ];
                ("Thinking Mode".into(), modes, PickerSource::ConfigSub("thinking".into()))
            }
            "effort" => {
                let levels = vec![
                    PickerItem { label: "low".into(), description: "Minimal thinking".into(), value: "low".into() },
                    PickerItem { label: "medium".into(), description: "Balanced".into(), value: "medium".into() },
                    PickerItem { label: "high".into(), description: "More thorough".into(), value: "high".into() },
                    PickerItem { label: "max".into(), description: "Maximum depth".into(), value: "max".into() },
                ];
                ("Effort Level".into(), levels, PickerSource::Effort)
            }
            "color" => {
                let colors = vec![
                    PickerItem { label: "blue".into(), description: "".into(), value: "blue".into() },
                    PickerItem { label: "green".into(), description: "".into(), value: "green".into() },
                    PickerItem { label: "cyan".into(), description: "".into(), value: "cyan".into() },
                    PickerItem { label: "yellow".into(), description: "".into(), value: "yellow".into() },
                    PickerItem { label: "red".into(), description: "".into(), value: "red".into() },
                    PickerItem { label: "magenta".into(), description: "".into(), value: "magenta".into() },
                    PickerItem { label: "white".into(), description: "".into(), value: "white".into() },
                ];
                ("Prompt Color".into(), colors, PickerSource::ConfigSub("color".into()))
            }
            _ => return,
        };

        let mut picker = PickerState::new(title, items, source);
        // Pre-select current value
        let current = match setting {
            "model" => self.engine.model.clone(),
            "theme" => self.engine.theme.clone(),
            "permissions" => self.engine.settings.permission_mode.clone().unwrap_or_default(),
            "thinking" => self.engine.settings.thinking.as_ref()
                .and_then(|t| t.mode.clone())
                .unwrap_or_default(),
            "effort" => self.get_effort_label().to_string(),
            "color" => self.engine.prompt_color.clone(),
            _ => String::new(),
        };
        if let Some(idx) = picker.items.iter().position(|i| i.value == current) {
            picker.selected = idx;
        }
        self.picker = Some(picker);
        self.input_mode = InputMode::Picker;
    }

    /// Open the diff viewer as a picker.
    async fn open_diff_viewer(&mut self) {
        // Get staged and unstaged diffs
        let staged = tokio::process::Command::new("git")
            .args(["diff", "--staged"])
            .current_dir(&self.engine.working_dir)
            .output()
            .await
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        let unstaged = tokio::process::Command::new("git")
            .args(["diff"])
            .current_dir(&self.engine.working_dir)
            .output()
            .await
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        let mut items = Vec::new();

        if staged.is_empty() && unstaged.is_empty() {
            self.messages.push(DisplayMessage::Status {
                text: "No changes to show.".to_string(),
            });
            return;
        }

        // Add staged changes header and lines
        if !staged.is_empty() {
            items.push(PickerItem {
                label: "── Staged Changes ──".into(),
                description: String::new(),
                value: String::new(),
            });
            for line in staged.lines().take(100) {
                let (label, desc) = if line.starts_with("+++") || line.starts_with("---") {
                    (line.to_string(), String::new())
                } else if line.starts_with("+") {
                    (line.to_string(), String::new())
                } else if line.starts_with("-") {
                    (line.to_string(), String::new())
                } else if line.starts_with("@@") {
                    (line.to_string(), String::new())
                } else {
                    (line.to_string(), String::new())
                };
                items.push(PickerItem { label, description: desc, value: String::new() });
            }
        }

        // Add unstaged changes header and lines
        if !unstaged.is_empty() {
            items.push(PickerItem {
                label: "── Unstaged Changes ──".into(),
                description: String::new(),
                value: String::new(),
            });
            for line in unstaged.lines().take(100) {
                items.push(PickerItem {
                    label: line.to_string(),
                    description: String::new(),
                    value: String::new(),
                });
            }
        }

        let picker = PickerState::new("Git Diff".into(), items, PickerSource::Help);
        self.picker = Some(picker);
        self.input_mode = InputMode::Picker;
    }

    /// Open the skills picker.
    fn open_skills_picker(&mut self) {
        let skills = self.skill_registry.as_ref()
            .map(|r| r.all())
            .unwrap_or_default();

        if skills.is_empty() {
            self.messages.push(DisplayMessage::Status {
                text: "No skills loaded.".to_string(),
            });
            return;
        }

        let items: Vec<PickerItem> = skills.iter().map(|s| {
            PickerItem {
                label: format!("/{}", s.name),
                description: s.description.clone(),
                value: s.name.clone(),
            }
        }).collect();

        let picker = PickerState::new("Skills".into(), items, PickerSource::Skills(Vec::new()));
        self.picker = Some(picker);
        self.input_mode = InputMode::Picker;
    }

    /// Open the help picker overlay.
    fn open_help_picker(&mut self) {
        let items = vec![
            PickerItem { label: "/help".into(), description: "Show this help".into(), value: "/help".into() },
            PickerItem { label: "/clear".into(), description: "Clear conversation history".into(), value: "/clear".into() },
            PickerItem { label: "/cost".into(), description: "Show token usage and cost".into(), value: "/cost".into() },
            PickerItem { label: "/model".into(), description: "Show or change model (Alt+P)".into(), value: "/model".into() },
            PickerItem { label: "/status".into(), description: "Show session status".into(), value: "/status".into() },
            PickerItem { label: "/compact".into(), description: "Compact conversation with AI".into(), value: "/compact".into() },
            PickerItem { label: "/diff".into(), description: "Show git diff".into(), value: "/diff".into() },
            PickerItem { label: "/commit".into(), description: "Generate commit message".into(), value: "/commit".into() },
            PickerItem { label: "/review".into(), description: "Review code changes".into(), value: "/review".into() },
            PickerItem { label: "/sessions".into(), description: "List saved sessions".into(), value: "/sessions".into() },
            PickerItem { label: "/resume".into(), description: "Resume a session".into(), value: "/resume".into() },
            PickerItem { label: "/fast".into(), description: "Toggle fast mode (Alt+O)".into(), value: "/fast".into() },
            PickerItem { label: "/permissions".into(), description: "Show permission mode (Shift+Tab)".into(), value: "/permissions".into() },
            PickerItem { label: "/config".into(), description: "Show configuration".into(), value: "/config".into() },
            PickerItem { label: "/theme".into(), description: "Set terminal theme".into(), value: "/theme".into() },
            PickerItem { label: "/effort".into(), description: "Set effort level".into(), value: "/effort".into() },
            PickerItem { label: "/quit".into(), description: "Exit Venus (Ctrl+D or Ctrl+C x2)".into(), value: "/quit".into() },
            PickerItem { label: "─── Shortcuts ───".into(), description: "".into(), value: "".into() },
            PickerItem { label: "Ctrl+L".into(), description: "Redraw screen".into(), value: "".into() },
            PickerItem { label: "Ctrl+R".into(), description: "Search history".into(), value: "".into() },
            PickerItem { label: "Ctrl+K".into(), description: "Delete to end of line".into(), value: "".into() },
            PickerItem { label: "Ctrl+U".into(), description: "Delete to start of line".into(), value: "".into() },
            PickerItem { label: "Ctrl+W".into(), description: "Delete word backward".into(), value: "".into() },
            PickerItem { label: "Alt+P".into(), description: "Open model picker".into(), value: "".into() },
            PickerItem { label: "Alt+T".into(), description: "Toggle thinking mode".into(), value: "".into() },
            PickerItem { label: "Alt+O".into(), description: "Toggle fast mode".into(), value: "".into() },
            PickerItem { label: "Shift+Tab".into(), description: "Cycle permission mode".into(), value: "".into() },
        ];
        let picker = PickerState::new("Commands & Shortcuts".into(), items, PickerSource::Help);
        self.picker = Some(picker);
        self.input_mode = InputMode::Picker;
    }

    /// Open history search mode.
    fn open_history_search(&mut self) {
        self.input.clear();
        self.input_mode = InputMode::HistorySearch;
    }

    /// Update history search filter based on current input buffer.
    fn history_search_update(&mut self) {
        // History search filtering happens at display time - we just keep the buffer
        // The matching entry is shown via the input display
        let query = self.input.buffer.to_lowercase();
        if query.is_empty() {
            return;
        }
        // Find most recent history entry matching query
        for entry in self.input.history.iter().rev() {
            if entry.to_lowercase().contains(&query) {
                // Replace buffer with matching entry for preview
                // Keep the query in a separate field would be ideal, but for simplicity
                // we'll just show the match in the input area
                break;
            }
        }
    }

    /// Toggle thinking mode.
    fn toggle_thinking(&mut self) {
        use venus_utils::config::ThinkingConfig;
        let current = self.engine.settings.thinking.as_ref()
            .and_then(|t| t.mode.as_deref())
            .unwrap_or("disabled");
        let next = match current {
            "disabled" => "enabled",
            "enabled" => "adaptive",
            _ => "disabled",
        };
        self.engine.settings = Arc::new({
            let mut s = (*self.engine.settings).clone();
            s.thinking = Some(ThinkingConfig {
                mode: Some(next.to_string()),
                budget_tokens: s.thinking.as_ref().and_then(|t| t.budget_tokens),
            });
            s
        });
        self.messages.push(DisplayMessage::Status {
            text: format!("Thinking mode: {}", next),
        });
    }

    /// Toggle fast mode (switch to/from haiku).
    fn toggle_fast_mode(&mut self) {
        let fast_model = "claude-haiku-4-5-20251001";
        if self.engine.model == fast_model {
            let original = self.engine.settings.effective_model().to_string();
            self.engine.model = original.clone();
            self.messages.push(DisplayMessage::Status {
                text: format!("Fast mode OFF -> {}", original),
            });
        } else {
            self.engine.model = fast_model.to_string();
            self.messages.push(DisplayMessage::Status {
                text: format!("Fast mode ON -> {}", fast_model),
            });
        }
    }

    /// Handle selection in the active picker.
    async fn handle_picker_select(&mut self) -> Result<()> {
        let picker = match self.picker.take() {
            Some(p) => p,
            None => return Ok(()),
        };
        let selected = match picker.selected_item() {
            Some(item) if !item.value.is_empty() => item.clone(),
            _ => {
                // Separator or empty value - just close picker
                self.input_mode = InputMode::Normal;
                return Ok(());
            }
        };

        match picker.source {
            PickerSource::Model => {
                self.engine.model = selected.value.clone();
                self.messages.push(DisplayMessage::Status {
                    text: format!("Model changed to: {}", selected.value),
                });
            }
            PickerSource::Theme => {
                self.engine.theme = selected.value.clone();
                self.messages.push(DisplayMessage::Status {
                    text: format!("Theme changed to: {}", selected.value),
                });
            }
            PickerSource::Help => {
                // Help picker just shows info, no action needed
            }
            PickerSource::Resume(sessions) => {
                if let Some(session) = sessions.iter().find(|s| s.id == selected.value) {
                    let session_id = session.id.clone();
                    match venus_utils::session::load_session(&session_id).await {
                        Ok((meta, msg_values)) => {
                            let messages: Vec<venus_core::message::Message> = msg_values
                                .iter()
                                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                                .collect();
                            let msg_count = messages.len();
                            *self.engine.messages.lock().await = messages;
                            self.engine.session_id = meta.id.clone();
                            self.engine.created_at = meta.created_at;
                            // Rebuild display messages from loaded conversation
                            self.messages.clear();
                            self.messages.push(DisplayMessage::Status {
                                text: format!("Resumed session {} ({} messages)", &meta.id[..8.min(meta.id.len())], msg_count),
                            });
                        }
                        Err(e) => {
                            self.messages.push(DisplayMessage::Error {
                                text: format!("Failed to load session: {}", e),
                            });
                        }
                    }
                }
            }
            PickerSource::Permissions => {
                if ["default", "auto", "bypass"].contains(&selected.value.as_str()) {
                    self.engine.settings = Arc::new({
                        let mut s = (*self.engine.settings).clone();
                        s.permission_mode = Some(selected.value.clone());
                        s
                    });
                    self.messages.push(DisplayMessage::Status {
                        text: format!("Permission mode: {}", selected.value),
                    });
                }
            }
            PickerSource::Effort => {
                use venus_utils::config::ThinkingConfig;
                let budget = match selected.value.as_str() {
                    "low" => Some(1024),
                    "medium" => Some(4096),
                    "high" => Some(10000),
                    "max" => Some(32000),
                    _ => None,
                };
                self.engine.settings = Arc::new({
                    let mut s = (*self.engine.settings).clone();
                    let existing_mode = s.thinking.as_ref().and_then(|t| t.mode.clone());
                    s.thinking = Some(ThinkingConfig {
                        mode: existing_mode.or_else(|| Some("enabled".to_string())),
                        budget_tokens: budget,
                    });
                    s
                });
                self.messages.push(DisplayMessage::Status {
                    text: format!("Effort level: {}", selected.value),
                });
            }
            PickerSource::Skills(_) => {
                // Skills picker invokes the selected skill
                return self.handle_slash_command(&format!("/{}", selected.value)).await;
            }
            PickerSource::Config => {
                // Config picker opens sub-picker for the selected setting
                self.open_config_sub_picker(&selected.value);
                return Ok(());
            }
            PickerSource::ConfigSub(setting) => {
                match setting.as_str() {
                    "thinking" => {
                        use venus_utils::config::ThinkingConfig;
                        self.engine.settings = Arc::new({
                            let mut s = (*self.engine.settings).clone();
                            s.thinking = Some(ThinkingConfig {
                                mode: Some(selected.value.clone()),
                                budget_tokens: s.thinking.as_ref().and_then(|t| t.budget_tokens),
                            });
                            s
                        });
                        self.messages.push(DisplayMessage::Status {
                            text: format!("Thinking mode: {}", selected.value),
                        });
                    }
                    "color" => {
                        self.engine.prompt_color = selected.value.clone();
                        self.messages.push(DisplayMessage::Status {
                            text: format!("Prompt color: {}", selected.value),
                        });
                    }
                    _ => {}
                }
            }
        }

        self.input_mode = InputMode::Normal;
        self.save_session();
        self.cost = self.engine.cost_tracker.lock().unwrap().format_cost();
        Ok(())
    }

    /// Update context window usage percentage.
    fn update_context_pct(&mut self) {
        // Use a blocking approach since we're in an async context
        let engine = self.engine.clone();
        let rt = tokio::runtime::Handle::current();
        let result = std::thread::spawn(move || {
            rt.block_on(async {
                let messages = engine.messages.lock().await;
                let analysis = venus_core::compact::analysis::analyze_context(
                    &messages,
                    &engine.system_prompt,
                );
                let window = venus_utils::context_window::context_window_for_model(&engine.model);
                if window > 0 {
                    (analysis.total_tokens as f64 / window as f64 * 100.0) as u64
                } else {
                    0
                }
            })
        }).join().unwrap_or(0);
        self.context_pct = result;
    }

    /// Open external editor for multi-line input.
    async fn open_external_editor(&mut self) {
        let editor = std::env::var("EDITOR")
            .or_else(|_| std::env::var("VISUAL"))
            .unwrap_or_else(|_| "vi".to_string());

        // Create temp file with current buffer content
        let temp_path = std::env::temp_dir().join("venus_input.md");
        let initial_content = if self.input.buffer.is_empty() {
            String::new()
        } else {
            self.input.buffer.clone()
        };
        if let Err(e) = std::fs::write(&temp_path, &initial_content) {
            self.messages.push(DisplayMessage::Error {
                text: format!("Failed to create temp file: {}", e),
            });
            return;
        }

        // Restore terminal before opening editor
        // We need to temporarily leave raw mode
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        );

        // Run editor
        let status = std::process::Command::new(&editor)
            .arg(&temp_path)
            .status();

        // Re-enter TUI mode
        let _ = crossterm::terminal::enable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableMouseCapture
        );

        match status {
            Ok(s) if s.success() => {
                match std::fs::read_to_string(&temp_path) {
                    Ok(content) => {
                        let trimmed = content.trim().to_string();
                        if !trimmed.is_empty() {
                            self.input.buffer = trimmed;
                            self.input.cursor_pos = self.input.buffer.len();
                        }
                    }
                    Err(e) => {
                        self.messages.push(DisplayMessage::Error {
                            text: format!("Failed to read editor output: {}", e),
                        });
                    }
                }
            }
            Ok(s) => {
                self.messages.push(DisplayMessage::Status {
                    text: format!("Editor exited with status: {}", s),
                });
            }
            Err(e) => {
                self.messages.push(DisplayMessage::Error {
                    text: format!("Failed to launch editor '{}': {}", editor, e),
                });
            }
        }

        // Clean up temp file
        let _ = std::fs::remove_file(&temp_path);
    }

    /// Get visible history entries matching the current search query.
    pub fn history_search_matches(&self) -> Vec<&str> {
        let query = self.input.buffer.to_lowercase();
        if query.is_empty() {
            return self.input.history.iter().rev().take(5).map(|s| s.as_str()).collect();
        }
        self.input.history.iter().rev()
            .filter(|entry| entry.to_lowercase().contains(&query))
            .take(5)
            .map(|s| s.as_str())
            .collect()
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
