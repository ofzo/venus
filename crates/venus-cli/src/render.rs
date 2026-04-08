use std::io::{self, Write};
use venus_core::engine::QueryEngine;
use venus_core::stream::StreamEvent;

use crate::markdown::MarkdownRenderer;

/// Newline for raw-mode terminal output.
const NL: &str = "\r\n";

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

pub fn render_event(event: &StreamEvent, md: &mut MarkdownRenderer) {
    match event {
        StreamEvent::TextDelta(text) => {
            md.push(text);
        }
        StreamEvent::ThinkingDelta(text) => {
            md.push_thinking(text);
        }
        StreamEvent::ToolUseStart { name, .. } => {
            let stderr = io::stderr();
            let mut out = stderr.lock();
            let _ = write!(out, "{}\x1b[36m  [Tool: {}]\x1b[0m{}", NL, name, NL);
        }
        StreamEvent::ToolUseInput(_) => {}
        StreamEvent::ToolResult { name, result, .. } => {
            let stderr = io::stderr();
            let mut out = stderr.lock();
            let status = if result.is_error {
                "\x1b[31merror\x1b[0m"
            } else {
                "\x1b[32mdone\x1b[0m"
            };
            let _ = write!(out, "  \x1b[36m[{}: {}]\x1b[0m{}", name, status, NL);

            for block in &result.content {
                if let venus_core::message::ContentBlock::Text { text } = block {
                    let display = if text.len() > 500 {
                        format!("{}...(truncated)", &text[..500])
                    } else {
                        text.clone()
                    };
                    for line in display.lines() {
                        let _ = write!(out, "  \x1b[2m{}\x1b[0m{}", line, NL);
                    }
                }
            }
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
    let cost = engine.cost_tracker.format_cost();
    let tokens = engine.cost_tracker.format_tokens();
    let _ = write!(out, "{}  Cost: {} | Tokens: {}{}{}", NL, cost, tokens, NL, NL);
}
