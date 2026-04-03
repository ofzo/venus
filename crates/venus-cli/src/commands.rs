use venus_core::engine::QueryEngine;

use crate::render;

/// Handle a slash command. Returns true if REPL should exit.
pub fn handle_command(input: &str, engine: &mut QueryEngine) -> bool {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0];
    let _args = parts.get(1).unwrap_or(&"");

    match cmd {
        "/exit" | "/quit" | "/q" => {
            eprintln!("Goodbye!");
            true
        }
        "/help" | "/h" => {
            print_help();
            false
        }
        "/clear" => {
            engine.messages.clear();
            eprintln!("  Conversation cleared.\n");
            false
        }
        "/cost" => {
            render::print_cost(engine);
            false
        }
        "/model" => {
            if let Some(model) = parts.get(1) {
                engine.model = model.to_string();
                eprintln!("  Model changed to: {}\n", model);
            } else {
                eprintln!("  Current model: {}\n", engine.model);
            }
            false
        }
        "/history" => {
            let count = engine.messages.len();
            eprintln!("  {} messages in conversation\n", count);
            false
        }
        _ => {
            eprintln!("  Unknown command: {}", cmd);
            eprintln!("  Type /help for available commands\n");
            false
        }
    }
}

fn print_help() {
    eprintln!(
        r#"
  Available commands:
    /help, /h       Show this help
    /exit, /quit    Exit the REPL
    /clear          Clear conversation history
    /cost           Show token usage and cost
    /model [name]   Show or change model
    /history        Show conversation message count

  Keyboard:
    Ctrl+C          Abort current query
    Ctrl+D          Exit
"#
    );
}
