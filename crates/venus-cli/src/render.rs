use venus_core::engine::QueryEngine;
use venus_core::stream::StreamEvent;

pub fn print_banner(engine: &QueryEngine) {
    eprintln!(
        "\x1b[1;34m  Venus\x1b[0m - model: \x1b[33m{}\x1b[0m",
        engine.model
    );
    eprintln!(
        "  working dir: {}",
        engine.working_dir.display()
    );
    eprintln!("  Type /help for commands, Ctrl+C to abort, Ctrl+D to exit\n");
}

pub fn render_event(event: &StreamEvent) {
    match event {
        StreamEvent::TextDelta(text) => {
            eprint!("{}", text);
        }
        StreamEvent::ThinkingDelta(text) => {
            eprint!("\x1b[2m{}\x1b[0m", text); // dimmed
        }
        StreamEvent::ToolUseStart { name, .. } => {
            eprintln!("\n\x1b[36m  [Tool: {}]\x1b[0m", name);
        }
        StreamEvent::ToolUseInput(_) => {
            // Don't display incremental JSON
        }
        StreamEvent::ToolResult { name, result, .. } => {
            let status = if result.is_error {
                "\x1b[31merror\x1b[0m"
            } else {
                "\x1b[32mdone\x1b[0m"
            };
            eprintln!("  \x1b[36m[{}: {}]\x1b[0m", name, status);

            // Show result content (truncated)
            for block in &result.content {
                if let venus_core::message::ContentBlock::Text { text } = block {
                    let display = if text.len() > 500 {
                        format!("{}...(truncated)", &text[..500])
                    } else {
                        text.clone()
                    };
                    for line in display.lines() {
                        eprintln!("  \x1b[2m{}\x1b[0m", line);
                    }
                }
            }
        }
        StreamEvent::MessageComplete(_) => {
            eprintln!(); // newline after response
        }
        StreamEvent::Error(err) => {
            eprintln!("\n\x1b[31mError: {}\x1b[0m", err);
        }
        StreamEvent::Usage(usage) => {
            let total = usage.input_tokens + usage.cache_read_tokens + usage.output_tokens;
            eprintln!(
                "\x1b[2m  tokens: {} (in:{} out:{})\x1b[0m",
                total, usage.input_tokens + usage.cache_read_tokens, usage.output_tokens
            );
        }
    }
}

pub fn print_cost(engine: &QueryEngine) {
    let cost = engine.cost_tracker.format_cost();
    let tokens = engine.cost_tracker.format_tokens();
    eprintln!("\n  Cost: {} | Tokens: {}\n", cost, tokens);
}
