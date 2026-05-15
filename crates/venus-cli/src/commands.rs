use std::sync::Arc;

use venus_core::engine::QueryEngine;
use venus_core::message::Message;
use venus_core::skill::SkillRegistry;
use venus_utils::session;

use crate::render;

/// Result of handling a slash command.
pub enum CommandResult {
    /// Continue the REPL normally.
    Continue,
    /// Exit the REPL.
    Exit,
    /// Inject a message as if the user typed it (used for skill invocation).
    InjectMessage(String),
}

/// Handle a slash command.
pub async fn handle_command(
    input: &str,
    engine: &mut QueryEngine,
    skill_registry: Option<&Arc<SkillRegistry>>,
) -> CommandResult {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0];
    let _args = parts.get(1).unwrap_or(&"");

    match cmd {
        "/exit" | "/quit" | "/q" => {
            eprintln!("Goodbye!");
            CommandResult::Exit
        }
        "/help" | "/h" => {
            print_help();
            CommandResult::Continue
        }
        "/clear" => {
            engine.messages.clear();
            eprintln!("  Conversation cleared.\n");
            CommandResult::Continue
        }
        "/cost" => {
            render::print_cost(engine);
            CommandResult::Continue
        }
        "/model" => {
            if let Some(model) = parts.get(1) {
                engine.model = model.to_string();
                eprintln!("  Model changed to: {}\n", model);
            } else {
                eprintln!("  Current model: {}\n", engine.model);
            }
            CommandResult::Continue
        }
        "/history" => {
            let count = engine.messages.len();
            eprintln!("  {} messages in conversation\n", count);
            CommandResult::Continue
        }
        "/diff" => {
            handle_diff(engine).await;
            CommandResult::Continue
        }
        "/compact" => {
            handle_compact(engine).await;
            CommandResult::Continue
        }
        "/config" => {
            handle_config(engine);
            CommandResult::Continue
        }
        "/doctor" => {
            handle_doctor(engine).await;
            CommandResult::Continue
        }
        "/context" => {
            handle_context(engine);
            CommandResult::Continue
        }
        "/tokens" => {
            handle_tokens(engine);
            CommandResult::Continue
        }
        "/plugin" | "/plugins" => {
            handle_plugins().await;
            CommandResult::Continue
        }
        "/sessions" => {
            let rt = tokio::runtime::Handle::current();
            let result = std::thread::spawn(move || rt.block_on(session::list_sessions()))
                .join()
                .unwrap_or_else(|_| Ok(Vec::new()));

            match result {
                Ok(sessions) => {
                    if sessions.is_empty() {
                        eprintln!("  No saved sessions.\n");
                    } else {
                        eprintln!("\n  Saved sessions:");
                        for (i, s) in sessions.iter().enumerate() {
                            let time = chrono::DateTime::from_timestamp(s.updated_at as i64, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_else(|| "unknown".to_string());
                            eprintln!(
                                "  {:>3}. {} | {} msgs | {} | {}",
                                i + 1,
                                &s.id[..8],
                                s.message_count,
                                s.model,
                                time,
                            );
                        }
                        eprintln!("\n  Use /resume <number> to resume a session.\n");
                    }
                }
                Err(e) => {
                    eprintln!("  Error listing sessions: {}\n", e);
                }
            }
            CommandResult::Continue
        }
        "/resume" => {
            let rt = tokio::runtime::Handle::current();
            let arg = parts.get(1).map(|s| s.trim().to_string());

            let sessions_result =
                std::thread::spawn({
                    let rt = rt.clone();
                    move || rt.block_on(session::list_sessions())
                })
                .join()
                .unwrap_or_else(|_| Ok(Vec::new()));

            let sessions = match sessions_result {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("  Error listing sessions: {}\n", e);
                    return CommandResult::Continue;
                }
            };

            if sessions.is_empty() {
                eprintln!("  No saved sessions.\n");
                return CommandResult::Continue;
            }

            let session_id = if let Some(ref a) = arg {
                if let Ok(idx) = a.parse::<usize>() {
                    if idx == 0 || idx > sessions.len() {
                        eprintln!("  Invalid session number. Use 1-{}.\n", sessions.len());
                        return CommandResult::Continue;
                    }
                    sessions[idx - 1].id.clone()
                } else {
                    match sessions.iter().find(|s| s.id.starts_with(a.as_str())) {
                        Some(s) => s.id.clone(),
                        None => {
                            eprintln!("  No session matching '{}' found.\n", a);
                            return CommandResult::Continue;
                        }
                    }
                }
            } else {
                eprintln!("\n  Recent sessions:");
                let display_count = sessions.len().min(10);
                for (i, s) in sessions.iter().take(display_count).enumerate() {
                    let time = chrono::DateTime::from_timestamp(s.updated_at as i64, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    eprintln!(
                        "  {:>3}. {} | {} msgs | {} | {}",
                        i + 1,
                        &s.id[..8],
                        s.message_count,
                        s.model,
                        time,
                    );
                }
                eprintln!("\n  Enter number to resume (or press Enter to cancel):");
                eprint!("  > ");
                std::io::Write::flush(&mut std::io::stderr()).ok();

                let mut choice = String::new();
                if std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut choice).is_err() {
                    return CommandResult::Continue;
                }
                let choice = choice.trim();
                if choice.is_empty() {
                    eprintln!("  Cancelled.\n");
                    return CommandResult::Continue;
                }
                match choice.parse::<usize>() {
                    Ok(idx) if idx >= 1 && idx <= display_count => {
                        sessions[idx - 1].id.clone()
                    }
                    _ => {
                        eprintln!("  Invalid choice.\n");
                        return CommandResult::Continue;
                    }
                }
            };

            let load_result = std::thread::spawn({
                let rt = rt.clone();
                let sid = session_id.clone();
                move || rt.block_on(session::load_session(&sid))
            })
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("thread panic")));

            match load_result {
                Ok((meta, msg_values)) => {
                    let messages: Vec<Message> = msg_values
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();
                    let msg_count = messages.len();
                    engine.messages = messages;
                    engine.session_id = meta.id.clone();
                    engine.created_at = meta.created_at;
                    eprintln!(
                        "  Resumed session {} ({} messages)\n",
                        &meta.id[..8],
                        msg_count,
                    );
                }
                Err(e) => {
                    eprintln!("  Error loading session: {}\n", e);
                }
            }
            CommandResult::Continue
        }
        _ => {
            // Check if it matches a user-invocable skill
            let skill_name = &cmd[1..]; // strip leading /
            if let Some(skill) = skill_registry
                .and_then(|r| r.find(skill_name))
                .filter(|s| s.user_invocable)
            {
                eprintln!("  Invoking skill: {}", skill.name);
                let content = if let Some(args) = parts.get(1) {
                    format!("{}\n\nArguments: {}", skill.content, args)
                } else {
                    skill.content.clone()
                };
                return CommandResult::InjectMessage(content);
            }

            eprintln!("  Unknown command: {}", cmd);
            eprintln!("  Type /help for available commands\n");
            CommandResult::Continue
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

/// Compact conversation using AI summarization, with fallback to naive truncation.
async fn handle_compact(engine: &mut QueryEngine) {
    let total = engine.messages.len();

    if total <= 4 {
        eprintln!("\n  Conversation has {} messages, nothing to compact.\n", total);
        return;
    }

    eprintln!("\n  Compacting conversation ({} messages)...", total);

    let config = venus_core::compact::CompactConfig::from_engine(
        &engine.model,
        engine.auth_header,
        &engine.auth_value,
        &engine.base_url,
    );

    match venus_core::compact::compact_with_hooks(
        &mut engine.messages,
        &config,
        Some(&engine.hook_runner),
        &engine.session_id,
    )
    .await
    {
        Ok(result) => {
            eprintln!(
                "  Compacted: {} -> {} messages (~{} tokens saved)\n",
                result.messages_before, result.messages_after, result.tokens_saved_estimate,
            );
        }
        Err(e) => {
            eprintln!("  AI summarization failed: {}", e);
            // Fallback to naive compaction
            let keep = 10.min(total);
            let removed = total - keep;
            engine.messages.drain(..removed);
            eprintln!(
                "  Fell back to keeping last {} messages (removed {}).\n",
                keep, removed
            );
        }
    }
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

    // Check API credentials
    let has_api_key = std::env::var("ANTHROPIC_API_KEY").is_ok();
    let has_auth_token = std::env::var("CLAUDE_CODE_OAUTH_TOKEN").is_ok()
        || std::env::var("ANTHROPIC_AUTH_TOKEN").is_ok();
    print_check("API credential (key or token)", has_api_key || has_auth_token);

    // Check ~/.claude/settings.json
    let settings_path = dirs_path("settings.json");
    let settings_exists = settings_path
        .map(|p| p.exists())
        .unwrap_or(false);
    print_check("~/.claude/settings.json", settings_exists);

    eprintln!();
}

/// Display rich context analysis with token breakdown.
fn handle_context(engine: &QueryEngine) {
    let analysis = venus_core::compact::analysis::analyze_context(
        &engine.messages,
        &engine.system_prompt,
    );
    let window = venus_utils::context_window::context_window_for_model(&engine.model);
    let threshold = venus_utils::context_window::auto_compact_threshold(&engine.model);

    let usage_pct = if window > 0 {
        (analysis.total_tokens as f64 / window as f64 * 100.0) as u64
    } else {
        0
    };

    eprintln!("\n  \x1b[1mContext info:\x1b[0m");
    eprintln!("    Model:               {}", engine.model);
    eprintln!("    Context window:      {} tokens", window);
    eprintln!("    Auto-compact at:     {} tokens", threshold);
    eprintln!("    Messages:            {}", analysis.message_count);
    eprintln!("    Turns:               {}", analysis.turn_count);
    eprintln!("    Usage:               ~{}% of context window", usage_pct);
    eprintln!();
    eprintln!("    \x1b[1mToken breakdown (estimated):\x1b[0m");
    eprintln!("      System prompt:     {}", analysis.system_prompt_tokens);
    eprintln!("      User text:         {}", analysis.user_text_tokens);
    eprintln!("      Assistant text:    {}", analysis.assistant_text_tokens);
    eprintln!(
        "      Tool requests:     {}",
        analysis.tool_request_tokens.values().sum::<u64>()
    );
    eprintln!(
        "      Tool results:      {}",
        analysis.tool_result_tokens.values().sum::<u64>()
    );
    eprintln!("      Thinking:          {}", analysis.thinking_tokens);
    eprintln!("      \x1b[1mTotal:             {}\x1b[0m", analysis.total_tokens);

    // Show per-tool breakdown if there are tool results
    if !analysis.tool_result_tokens.is_empty() {
        eprintln!();
        eprintln!("    \x1b[1mTool result tokens:\x1b[0m");
        let mut sorted: Vec<_> = analysis.tool_result_tokens.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (name, tokens) in sorted {
            eprintln!("      {:<20} {}", name, tokens);
        }
    }

    if !analysis.duplicate_file_reads.is_empty() {
        eprintln!();
        eprintln!("    \x1b[33mDuplicate file reads:\x1b[0m");
        for (path, count) in &analysis.duplicate_file_reads {
            eprintln!("      {} ({}x)", path, count);
        }
    }

    eprintln!();
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

/// List installed plugins from standard directories.
async fn handle_plugins() {
    let plugin_dirs = vec![
        dirs::home_dir()
            .unwrap_or_default()
            .join(".claude")
            .join("plugins"),
        std::env::current_dir()
            .unwrap_or_default()
            .join(".claude")
            .join("plugins"),
    ];

    let mut registry = venus_core::plugin_registry::PluginRegistry::new();
    if let Err(e) = registry.load_all(&plugin_dirs).await {
        eprintln!("  Error loading plugins: {}\n", e);
        return;
    }

    let plugins = registry.all_plugins();
    if plugins.is_empty() {
        eprintln!(
            "\n  No plugins installed.\n\n  Place plugins in ~/.claude/plugins/ or ./.claude/plugins/.\n  Each plugin directory must contain a plugin.json manifest.\n"
        );
        return;
    }

    eprintln!("\n  \x1b[1mInstalled plugins:\x1b[0m");
    for plugin in plugins {
        let desc = plugin
            .manifest
            .description
            .as_deref()
            .unwrap_or("(no description)");
        eprintln!(
            "    \x1b[33m{}\x1b[0m v{} - {}",
            plugin.manifest.name, plugin.manifest.version, desc
        );
        if !plugin.manifest.tools.is_empty() {
            let tool_names: Vec<&str> =
                plugin.manifest.tools.iter().map(|t| t.name.as_str()).collect();
            eprintln!("      Tools: {}", tool_names.join(", "));
        }
        if !plugin.manifest.mcp_servers.is_empty() {
            let server_names: Vec<&str> = plugin.manifest.mcp_servers.keys().map(|s| s.as_str()).collect();
            eprintln!("      MCP servers: {}", server_names.join(", "));
        }
        if !plugin.manifest.commands.is_empty() {
            let cmd_names: Vec<&str> =
                plugin.manifest.commands.iter().map(|c| c.name.as_str()).collect();
            eprintln!("      Commands: {}", cmd_names.join(", "));
        }
    }
    eprintln!();
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
    /compact        Compact conversation with AI summarization
    /config         Show current configuration
    /doctor         Run environment diagnostics
    /context        Show context info
    /tokens         Show detailed token breakdown
    /plugin         List installed plugins
    /sessions       List all saved sessions
    /resume [n|id]  Resume a previous session

  Keyboard:
    Ctrl+C          Abort current query
    Ctrl+D          Exit
"#
    );
}
