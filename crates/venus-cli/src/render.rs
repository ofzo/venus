use std::io::{self, Write};
use venus_core::engine::QueryEngine;
use venus_core::stream::StreamEvent;

use crate::markdown::MarkdownRenderer;

/// Newline for raw-mode terminal output.
const NL: &str = "\r\n";

/// Output format for rendered events.
#[derive(clap::ValueEnum, Clone, Debug, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    StreamJson,
}

pub fn print_banner(engine: &QueryEngine) {
    let stderr = io::stderr();
    let mut out = stderr.lock();
    let _ = write!(
        out,
        "\x1b[1;34m  Venus\x1b[0m - model: \x1b[33m{}\x1b[0m{}",
        engine.model, NL
    );
    let _ = write!(
        out,
        "  working dir: {}{}",
        engine.working_dir.display(),
        NL
    );
    let _ = write!(
        out,
        "  Type /help for commands, Ctrl+C to abort, Ctrl+D to exit{}{}",
        NL, NL
    );
}

/// State for rendering, tracking active tool input for activity descriptions.
pub struct RenderState {
    active_tool_name: String,
    active_tool_input: String,
    activity_shown: bool,
    thinking_active: bool,
}

impl RenderState {
    pub fn new() -> Self {
        Self {
            active_tool_name: String::new(),
            active_tool_input: String::new(),
            activity_shown: false,
            thinking_active: false,
        }
    }

    /// Show "Thinking..." indicator. Call after submitting a message.
    pub fn show_thinking(&mut self) {
        let stderr = io::stderr();
        let mut out = stderr.lock();
        let _ = write!(out, "  \x1b[2mThinking...\x1b[0m{}", NL);
        self.thinking_active = true;
    }

    /// Clear the thinking indicator. Called automatically on first content event.
    fn clear_thinking(&mut self) {
        if self.thinking_active {
            // Move cursor up one line and clear it to remove "Thinking..."
            let stderr = io::stderr();
            let mut out = stderr.lock();
            let _ = write!(out, "\x1b[1A\x1b[2K");
            self.thinking_active = false;
        }
    }
}

/// Extract a human-readable activity description from tool name + JSON input.
fn tool_activity(tool_name: &str, input_json: &str) -> Option<String> {
    let input: serde_json::Value = serde_json::from_str(input_json).ok()?;
    match tool_name {
        "Bash" => {
            let cmd = input.get("command")?.as_str()?;
            let display = if cmd.len() > 80 { &cmd[..80] } else { cmd };
            Some(display.to_string())
        }
        "Read" | "Write" | "Edit" | "FileReadTool" | "FileWriteTool" | "FileEditTool" => {
            let path = input.get("file_path")?.as_str()?;
            Some(path.to_string())
        }
        "Glob" | "GlobTool" => {
            let pattern = input.get("pattern")?.as_str()?;
            Some(pattern.to_string())
        }
        "Grep" | "GrepTool" => {
            let pattern = input.get("pattern")?.as_str()?;
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            Some(format!("{} in {}", pattern, path))
        }
        "Agent" | "AgentTool" => {
            let desc = input.get("description")?.as_str()?;
            Some(desc.to_string())
        }
        "WebFetch" | "WebFetchTool" => {
            let url = input.get("url")?.as_str()?;
            Some(url.to_string())
        }
        "WebSearch" | "WebSearchTool" => {
            let query = input.get("query")?.as_str()?;
            Some(query.to_string())
        }
        "LspTool" | "LSPTool" => {
            let op = input.get("operation")?.as_str()?;
            let file = input.get("filePath").and_then(|v| v.as_str()).unwrap_or("");
            Some(format!("{} {}", op, file))
        }
        _ => None,
    }
}

pub fn render_event(event: &StreamEvent, md: &mut MarkdownRenderer, state: &mut RenderState) {
    match event {
        StreamEvent::TextDelta(text) => {
            state.clear_thinking();
            md.push(text);
        }
        StreamEvent::ThinkingDelta(text) => {
            state.clear_thinking();
            md.push_thinking(text);
        }
        StreamEvent::ToolUseStart { name, .. } => {
            state.active_tool_name = name.clone();
            state.active_tool_input.clear();
            state.activity_shown = false;
            let stderr = io::stderr();
            let mut out = stderr.lock();
            let _ = write!(out, "{}\x1b[1;36m  > {}\x1b[0m", NL, name);
        }
        StreamEvent::ToolUseInput(json) => {
            if !state.activity_shown {
                state.active_tool_input.push_str(json);
                if let Some(activity) = tool_activity(&state.active_tool_name, &state.active_tool_input) {
                    let stderr = io::stderr();
                    let mut out = stderr.lock();
                    let _ = write!(out, " \x1b[2m{}\x1b[0m{}", activity, NL);
                    state.activity_shown = true;
                }
            }
        }
        StreamEvent::ToolResult { name, result, .. } => {
            let stderr = io::stderr();
            let mut out = stderr.lock();
            // Add newline after tool header if no activity description was shown
            if !state.activity_shown {
                let _ = write!(out, "{}", NL);
            }
            state.active_tool_name.clear();
            state.active_tool_input.clear();
            state.activity_shown = false;
            let status = if result.is_error {
                "\x1b[31merror\x1b[0m"
            } else {
                "\x1b[32mdone\x1b[0m"
            };
            let _ = write!(out, "  \x1b[36m{}\x1b[0m {}{}", name, status, NL);

            for block in &result.content {
                if let venus_core::message::ContentBlock::Text { text } = block {
                    let display = if text.len() > 500 {
                        format!("{}...(truncated)", &text[..500])
                    } else {
                        text.clone()
                    };
                    for line in display.lines() {
                        let _ = write!(out, "    \x1b[2m{}\x1b[0m{}", line, NL);
                    }
                }
            }
            let _ = write!(out, "{}", NL);
        }
        StreamEvent::MessageComplete(_) => {
            md.finish();
            let stderr = io::stderr();
            let mut out = stderr.lock();
            let _ = write!(out, "{}", NL);
        }
        StreamEvent::Error(err) => {
            let stderr = io::stderr();
            let mut out = stderr.lock();
            let _ = write!(out, "{}\x1b[31mError: {}\x1b[0m{}", NL, err, NL);
        }
        StreamEvent::Usage(usage) => {
            let stderr = io::stderr();
            let mut out = stderr.lock();
            let total = usage.input_tokens + usage.cache_read_tokens + usage.output_tokens;
            let _ = write!(
                out,
                "\x1b[2m  tokens: {} (in:{} out:{})\x1b[0m{}",
                total,
                usage.input_tokens + usage.cache_read_tokens,
                usage.output_tokens,
                NL
            );
        }
        StreamEvent::AutoCompacted {
            messages_removed,
            tokens_saved,
        } => {
            let stderr = io::stderr();
            let mut out = stderr.lock();
            let _ = write!(
                out,
                "\x1b[2m  [auto-compacted: removed {} messages, ~{} tokens saved]\x1b[0m{}",
                messages_removed, tokens_saved, NL
            );
        }
    }
}

pub fn print_cost(engine: &QueryEngine) {
    let stderr = io::stderr();
    let mut out = stderr.lock();
    let tracker = engine.cost_tracker.lock().unwrap();
    let cost = tracker.format_cost();
    let tokens = tracker.format_tokens();
    let _ = write!(out, "{}  Cost: {} | Tokens: {}{}{}", NL, cost, tokens, NL, NL);
}

/// Convert a StreamEvent to a serde_json::Value for JSON output.
fn event_to_json(event: &StreamEvent) -> serde_json::Value {
    match event {
        StreamEvent::TextDelta(text) => {
            serde_json::json!({"type": "text_delta", "text": text})
        }
        StreamEvent::ThinkingDelta(text) => {
            serde_json::json!({"type": "thinking_delta", "text": text})
        }
        StreamEvent::ToolUseStart { id, name } => {
            serde_json::json!({"type": "tool_use_start", "id": id, "name": name})
        }
        StreamEvent::ToolUseInput(input) => {
            serde_json::json!({"type": "tool_use_input", "input": input})
        }
        StreamEvent::ToolResult { id, name, result } => {
            let content: Vec<String> = result.content.iter().filter_map(|b| {
                if let venus_core::message::ContentBlock::Text { text } = b {
                    Some(text.clone())
                } else {
                    None
                }
            }).collect();
            serde_json::json!({
                "type": "tool_result",
                "id": id,
                "name": name,
                "is_error": result.is_error,
                "content": content,
            })
        }
        StreamEvent::MessageComplete(msg) => {
            serde_json::json!({"type": "message_complete", "role": "assistant", "stop_reason": msg.stop_reason})
        }
        StreamEvent::Error(err) => {
            serde_json::json!({"type": "error", "error": err})
        }
        StreamEvent::Usage(usage) => {
            serde_json::json!({
                "type": "usage",
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_read_tokens": usage.cache_read_tokens,
            })
        }
        StreamEvent::AutoCompacted { messages_removed, tokens_saved } => {
            serde_json::json!({
                "type": "auto_compacted",
                "messages_removed": messages_removed,
                "tokens_saved": tokens_saved,
            })
        }
    }
}

/// Render a single event as an NDJSON line to stdout.
pub fn render_event_ndjson(event: &StreamEvent) {
    let json = event_to_json(event);
    if let Ok(line) = serde_json::to_string(&json) {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        let _ = writeln!(out, "{}", line);
    }
}

/// Collect all events for JSON output mode.
pub struct JsonCollector {
    events: Vec<serde_json::Value>,
}

impl JsonCollector {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    pub fn push(&mut self, event: &StreamEvent) {
        self.events.push(event_to_json(event));
    }

    pub fn finish(self) {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        if let Ok(json) = serde_json::to_string_pretty(&self.events) {
            let _ = writeln!(out, "{}", json);
        }
    }
}
