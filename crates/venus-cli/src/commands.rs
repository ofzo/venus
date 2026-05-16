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
            engine.messages.lock().await.clear();
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
            let count = engine.messages.lock().await.len();
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
            handle_config(engine).await;
            CommandResult::Continue
        }
        "/doctor" => {
            handle_doctor(engine).await;
            CommandResult::Continue
        }
        "/context" => {
            handle_context(engine).await;
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
                    *engine.messages.lock().await = messages;
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
        "/commit" => {
            let diff_output = get_staged_diff(&engine.working_dir).await;
            if diff_output.is_empty() {
                eprintln!("  No staged changes. Use `git add` first.\n");
                return CommandResult::Continue;
            }
            let prompt = format!(
                "Analyze the following staged changes and create a conventional commit message.\n\
                 Follow conventional commits format (type(scope): description).\n\
                 Only output the commit message, nothing else.\n\n\
                 ```diff\n{}\n```",
                diff_output
            );
            CommandResult::InjectMessage(prompt)
        }
        "/review" => {
            let diff_output = get_full_diff(&engine.working_dir).await;
            if diff_output.is_empty() {
                eprintln!("  No changes to review.\n");
                return CommandResult::Continue;
            }
            let prompt = format!(
                "Review the following code changes. Focus on bugs, security issues, \
                 performance problems, and code quality.\n\n```diff\n{}\n```",
                diff_output
            );
            CommandResult::InjectMessage(prompt)
        }
        "/init" => {
            let claude_md = engine.working_dir.join("CLAUDE.md");
            if claude_md.exists() {
                eprintln!("  CLAUDE.md already exists.\n");
                return CommandResult::Continue;
            }
            CommandResult::InjectMessage(
                "Create a CLAUDE.md file for this project. Analyze the project structure, \
                 language, build system, and conventions. Include: project overview, \
                 build/test commands, code style, important notes.".to_string()
            )
        }
        "/memory" => {
            let arg = parts.get(1).unwrap_or(&"").trim();
            if arg.is_empty() || arg == "list" {
                match venus_utils::memory::list_memories(None, Some(&engine.working_dir)).await {
                    Ok(entries) if entries.is_empty() => eprintln!("  No memory entries.\n"),
                    Ok(entries) => {
                        eprintln!("\n  Memory entries:");
                        for e in &entries {
                            eprintln!(
                                "    [{}] {} ({})",
                                &e.id[..8.min(e.id.len())],
                                e.title,
                                e.memory_type
                            );
                        }
                        eprintln!();
                    }
                    Err(e) => eprintln!("  Error: {}\n", e),
                }
            } else {
                eprintln!("  Usage: /memory [list]\n");
            }
            CommandResult::Continue
        }
        "/skills" => {
            if let Some(registry) = skill_registry {
                let all = registry.all();
                if all.is_empty() {
                    eprintln!("  No skills loaded.\n");
                } else {
                    eprintln!("\n  Loaded skills:");
                    for s in all {
                        eprintln!("    /{} - {}", s.name, s.description);
                    }
                    eprintln!();
                }
            } else {
                eprintln!("  Skill registry not available.\n");
            }
            CommandResult::Continue
        }
        "/tasks" => {
            let tasks = engine.task_store.list();
            if tasks.is_empty() {
                eprintln!("  No active tasks.\n");
            } else {
                eprintln!("\n  Tasks:");
                for t in &tasks {
                    let icon = match t.status {
                        venus_core::task::TaskStatus::Pending => "○",
                        venus_core::task::TaskStatus::InProgress => "◉",
                        venus_core::task::TaskStatus::Completed => "●",
                        venus_core::task::TaskStatus::Deleted => "✗",
                    };
                    eprintln!("    {} {} - {}", icon, t.id, t.subject);
                }
                eprintln!();
            }
            CommandResult::Continue
        }
        "/plan" => {
            let current = engine.plan_mode.load(std::sync::atomic::Ordering::Relaxed);
            let new_val = !current;
            engine.plan_mode.store(new_val, std::sync::atomic::Ordering::Relaxed);
            eprintln!("  Plan mode: {}\n", if new_val { "ON" } else { "OFF" });
            CommandResult::Continue
        }
        "/vim" => {
            eprintln!("  Vim mode toggle (reedline integration pending)\n");
            CommandResult::Continue
        }
        "/effort" => {
            if let Some(level) = parts.get(1) {
                match level.trim() {
                    "low" | "medium" | "high" | "max" => {
                        eprintln!("  Effort level set to: {}\n", level.trim());
                    }
                    _ => eprintln!("  Usage: /effort [low|medium|high|max]\n"),
                }
            } else {
                eprintln!("  Usage: /effort [low|medium|high|max]\n");
            }
            CommandResult::Continue
        }
        "/copy" => {
            let messages = engine.messages.lock().await;
            let last = messages.iter().rev().find_map(|m| {
                if let venus_core::message::Message::Assistant(a) = m { Some(a) } else { None }
            });
            if let Some(msg) = last {
                let text: String = msg.content.iter()
                    .filter_map(|b| b.as_text())
                    .collect::<Vec<_>>()
                    .join("\n");
                #[cfg(target_os = "macos")]
                {
                    use std::io::Write;
                    if let Ok(mut child) = std::process::Command::new("pbcopy")
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                    {
                        if let Some(ref mut stdin) = child.stdin {
                            let _ = stdin.write_all(text.as_bytes());
                        }
                        let _ = child.wait();
                        eprintln!("  Copied to clipboard.\n");
                    }
                }
                #[cfg(not(target_os = "macos"))]
                eprintln!("  Clipboard not available on this platform.\n");
            } else {
                eprintln!("  No assistant message to copy.\n");
            }
            CommandResult::Continue
        }
        "/version" => {
            eprintln!("  Venus v{}", env!("CARGO_PKG_VERSION"));
            eprintln!("  Model: {}", engine.model);
            eprintln!();
            CommandResult::Continue
        }
        "/status" => {
            let uptime = chrono::Utc::now().timestamp() as u64 - engine.created_at;
            let cost = engine.cost_tracker.lock().unwrap().format_cost();
            let msg_count = engine.messages.lock().await.len();
            eprintln!("\n  Session status:");
            eprintln!("    Uptime:     {}s", uptime);
            eprintln!("    Messages:   {}", msg_count);
            eprintln!("    Cost:       {}", cost);
            eprintln!("    Model:      {}", engine.model);
            eprintln!("    Plan mode:  {}", engine.plan_mode.load(std::sync::atomic::Ordering::Relaxed));
            eprintln!();
            CommandResult::Continue
        }
        "/summary" => {
            if engine.messages.lock().await.len() < 4 {
                eprintln!("  Not enough messages to summarize.\n");
                return CommandResult::Continue;
            }
            CommandResult::InjectMessage(
                "Provide a brief summary of our conversation so far.".to_string()
            )
        }
        "/export" => {
            let path = parts.get(1).map(|s| s.trim()).unwrap_or("conversation.json");
            let messages = engine.messages.lock().await;
            let values: Vec<serde_json::Value> = messages.iter()
                .filter_map(|m| serde_json::to_value(m).ok())
                .collect();
            drop(messages);
            match serde_json::to_string_pretty(&values) {
                Ok(json) => match std::fs::write(path, &json) {
                    Ok(()) => eprintln!("  Exported {} messages to {}\n", values.len(), path),
                    Err(e) => eprintln!("  Error writing {}: {}\n", path, e),
                },
                Err(e) => eprintln!("  Error serializing: {}\n", e),
            }
            CommandResult::Continue
        }
        "/rewind" => {
            let n: usize = parts.get(1).and_then(|s| s.trim().parse().ok()).unwrap_or(1);
            let mut messages = engine.messages.lock().await;
            let total = messages.len();
            let remove = (n * 2).min(total);
            if remove == 0 {
                eprintln!("  Nothing to rewind.\n");
            } else {
                messages.drain((total - remove)..);
                eprintln!("  Rewound {} messages.\n", remove);
            }
            CommandResult::Continue
        }
        "/permissions" => {
            let mode = engine.settings.permission_mode.as_deref().unwrap_or("default");
            eprintln!("\n  Permission mode: {}", mode);
            if let Some(ref allow) = engine.settings.always_allow {
                for rule in allow {
                    eprintln!("    ALLOW  {}:{}", rule.tool, rule.pattern.as_deref().unwrap_or("*"));
                }
            }
            if let Some(ref deny) = engine.settings.always_deny {
                for rule in deny {
                    eprintln!("    DENY   {}:{}", rule.tool, rule.pattern.as_deref().unwrap_or("*"));
                }
            }
            eprintln!();
            CommandResult::Continue
        }
        "/mcp" => {
            if let Some(ref servers) = engine.settings.mcp_servers {
                if servers.is_empty() {
                    eprintln!("  No MCP servers configured.\n");
                } else {
                    eprintln!("\n  MCP servers:");
                    for (name, config) in servers {
                        eprintln!("    {} - {} ({})", name, config.command, config.transport);
                    }
                    eprintln!();
                }
            } else {
                eprintln!("  No MCP servers configured.\n");
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
async fn handle_compact(engine: &QueryEngine) {
    let total = engine.messages.lock().await.len();

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

    let mut messages = engine.messages.lock().await;
    match venus_core::compact::compact_with_hooks(
        &mut messages,
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
            messages.drain(..removed);
            eprintln!(
                "  Fell back to keeping last {} messages (removed {}).\n",
                keep, removed
            );
        }
    }
}

/// Display current configuration.
async fn handle_config(engine: &QueryEngine) {
    let permission_mode = engine
        .settings
        .permission_mode
        .as_deref()
        .unwrap_or("default");

    let total_usage = engine.cost_tracker.lock().unwrap().total_usage();

    eprintln!("\n  \x1b[1mConfiguration:\x1b[0m");
    eprintln!("    Model:           {}", engine.model);
    eprintln!("    Base URL:        {}", engine.base_url);
    eprintln!("    Working dir:     {}", engine.working_dir.display());
    eprintln!("    Permission mode: {}", permission_mode);
    eprintln!("    Max tokens:      {}", engine.max_tokens);
    eprintln!("    Max turns:       {}", engine.max_turns);
    if let Some(budget) = engine.budget_usd {
        eprintln!("    Budget:          ${:.2}", budget);
    }
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
async fn handle_context(engine: &QueryEngine) {
    let messages = engine.messages.lock().await;
    let analysis = venus_core::compact::analysis::analyze_context(
        &messages,
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

    let tracker = engine.cost_tracker.lock().unwrap();
    if tracker.usage_by_model.is_empty() {
        eprintln!("    No token usage recorded yet.\n");
        return;
    }

    for (model, usage) in &tracker.usage_by_model {
        eprintln!("    \x1b[33m{}\x1b[0m", model);
        eprintln!("      Input tokens:          {}", usage.input_tokens);
        eprintln!("      Output tokens:         {}", usage.output_tokens);
        eprintln!("      Cache read tokens:     {}", usage.cache_read_tokens);
        eprintln!("      Cache creation tokens: {}", usage.cache_creation_tokens);
    }

    let cost = tracker.format_cost();
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
    /commit         Generate conventional commit from staged changes
    /review         Review code changes for issues
    /init           Create CLAUDE.md for this project
    /memory [list]  List memory entries
    /skills         List loaded skills
    /tasks          List active tasks
    /plan           Toggle plan mode
    /vim            Toggle vim mode (pending)
    /effort [level] Set effort level (low/medium/high/max)
    /copy           Copy last assistant message to clipboard
    /version        Show version and model info
    /status         Show session status
    /summary        Summarize conversation
    /export [path]  Export conversation to JSON
    /rewind [n]     Rewind n message pairs
    /permissions    Show permission rules
    /mcp            Show MCP server config

  Keyboard:
    Ctrl+C          Abort current query
    Ctrl+D          Exit
"#
    );
}

async fn get_staged_diff(working_dir: &std::path::Path) -> String {
    tokio::process::Command::new("git")
        .args(["diff", "--staged"])
        .current_dir(working_dir)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

async fn get_full_diff(working_dir: &std::path::Path) -> String {
    let staged = tokio::process::Command::new("git")
        .args(["diff", "--staged"])
        .current_dir(working_dir)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let unstaged = tokio::process::Command::new("git")
        .args(["diff"])
        .current_dir(working_dir)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    format!("{}\n{}", staged, unstaged)
}
