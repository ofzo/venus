use venus_core::engine::QueryEngine;

use crate::render;

/// Handle a slash command. Returns true if REPL should exit.
pub async fn handle_command(input: &str, engine: &mut QueryEngine) -> bool {
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
        "/diff" => {
            handle_diff(engine).await;
            false
        }
        "/compact" => {
            handle_compact(engine);
            false
        }
        "/config" => {
            handle_config(engine);
            false
        }
        "/doctor" => {
            handle_doctor(engine).await;
            false
        }
        "/context" => {
            handle_context(engine);
            false
        }
        "/tokens" => {
            handle_tokens(engine);
            false
        }
        _ => {
            eprintln!("  Unknown command: {}", cmd);
            eprintln!("  Type /help for available commands\n");
            false
        }
    }
}

/// Run `git diff` and `git diff --staged` in the working directory and display output.
async fn handle_diff(engine: &QueryEngine) {
    eprintln!();

    // Unstaged changes
    match tokio::process::Command::new("git")
        .args(["diff"])
        .current_dir(&engine.working_dir)
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.is_empty() {
                eprintln!("  No unstaged changes.");
            } else {
                eprintln!("  \x1b[1mUnstaged changes:\x1b[0m");
                for line in stdout.lines() {
                    eprintln!("  {}", line);
                }
            }
        }
        Err(e) => {
            eprintln!("  \x1b[31mFailed to run git diff: {}\x1b[0m", e);
        }
    }

    // Staged changes
    match tokio::process::Command::new("git")
        .args(["diff", "--staged"])
        .current_dir(&engine.working_dir)
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.is_empty() {
                eprintln!("  No staged changes.");
            } else {
                eprintln!("\n  \x1b[1mStaged changes:\x1b[0m");
                for line in stdout.lines() {
                    eprintln!("  {}", line);
                }
            }
        }
        Err(e) => {
            eprintln!("  \x1b[31mFailed to run git diff --staged: {}\x1b[0m", e);
        }
    }

    eprintln!();
}

/// Keep only the last N messages, dropping older ones to reduce context size.
fn handle_compact(engine: &mut QueryEngine) {
    const KEEP_LAST: usize = 10;
    let total = engine.messages.len();

    if total <= KEEP_LAST {
        eprintln!("\n  Conversation has {} messages, nothing to compact.\n", total);
        return;
    }

    let removed = total - KEEP_LAST;
    engine.messages.drain(..removed);
    eprintln!(
        "\n  Compacted conversation: removed {} old messages, kept last {}.\n",
        removed, KEEP_LAST
    );
}

/// Display current configuration.
fn handle_config(engine: &QueryEngine) {
    let permission_mode = engine
        .settings
        .permission_mode
        .as_deref()
        .unwrap_or("default");

    let total_usage = engine.cost_tracker.total_usage();

    eprintln!("\n  \x1b[1mConfiguration:\x1b[0m");
    eprintln!("    Model:           {}", engine.model);
    eprintln!("    Base URL:        {}", engine.base_url);
    eprintln!("    Working dir:     {}", engine.working_dir.display());
    eprintln!("    Permission mode: {}", permission_mode);
    eprintln!("    Max tokens:      {}", engine.max_tokens);
    eprintln!(
        "    Token usage:     {} input, {} output\n",
        total_usage.input_tokens + total_usage.cache_read_tokens,
        total_usage.output_tokens
    );
}

/// Run environment diagnostics, checking for required tools and config.
async fn handle_doctor(engine: &QueryEngine) {
    eprintln!("\n  \x1b[1mEnvironment diagnostics:\x1b[0m");

    // Check git
    let git_ok = check_command("git", &["--version"], &engine.working_dir).await;
    print_check("git", git_ok);

    // Check ripgrep
    let rg_ok = check_command("rg", &["--version"], &engine.working_dir).await;
    print_check("rg (ripgrep)", rg_ok);

    // Check ANTHROPIC_API_KEY
    let api_key_set = std::env::var("ANTHROPIC_API_KEY").is_ok();
    print_check("ANTHROPIC_API_KEY", api_key_set);

    // Check ~/.claude/settings.json
    let settings_path = dirs_path("settings.json");
    let settings_exists = settings_path
        .map(|p| p.exists())
        .unwrap_or(false);
    print_check("~/.claude/settings.json", settings_exists);

    eprintln!();
}

/// Display conversation context info.
fn handle_context(engine: &QueryEngine) {
    let msg_count = engine.messages.len();
    let total_usage = engine.cost_tracker.total_usage();
    let total_tokens = total_usage.input_tokens
        + total_usage.output_tokens
        + total_usage.cache_read_tokens
        + total_usage.cache_creation_tokens;
    let system_len = engine.system_prompt.len();

    eprintln!("\n  \x1b[1mContext info:\x1b[0m");
    eprintln!("    Messages:            {}", msg_count);
    eprintln!("    Total tokens used:   {}", total_tokens);
    eprintln!("    System prompt chars: {}\n", system_len);
}

/// Display detailed per-model token breakdown.
fn handle_tokens(engine: &QueryEngine) {
    eprintln!("\n  \x1b[1mToken breakdown:\x1b[0m");

    if engine.cost_tracker.usage_by_model.is_empty() {
        eprintln!("    No token usage recorded yet.\n");
        return;
    }

    for (model, usage) in &engine.cost_tracker.usage_by_model {
        eprintln!("    \x1b[33m{}\x1b[0m", model);
        eprintln!("      Input tokens:          {}", usage.input_tokens);
        eprintln!("      Output tokens:         {}", usage.output_tokens);
        eprintln!("      Cache read tokens:     {}", usage.cache_read_tokens);
        eprintln!("      Cache creation tokens: {}", usage.cache_creation_tokens);
    }

    let cost = engine.cost_tracker.format_cost();
    eprintln!("\n    Total cost: {}\n", cost);
}

/// Check if a command is available by running it.
async fn check_command(cmd: &str, args: &[&str], working_dir: &std::path::Path) -> bool {
    tokio::process::Command::new(cmd)
        .args(args)
        .current_dir(working_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Print a pass/fail check line.
fn print_check(label: &str, ok: bool) {
    let icon = if ok {
        "\x1b[32mpass\x1b[0m"
    } else {
        "\x1b[31mfail\x1b[0m"
    };
    eprintln!("    [{}] {}", icon, label);
}

/// Resolve ~/.claude/<filename> path.
fn dirs_path(filename: &str) -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join(filename))
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
    /diff           Show git diff (staged + unstaged)
    /compact        Compact conversation (keep last 10 messages)
    /config         Show current configuration
    /doctor         Run environment diagnostics
    /context        Show context info
    /tokens         Show detailed token breakdown

  Keyboard:
    Ctrl+C          Abort current query
    Ctrl+D          Exit
"#
    );
}
