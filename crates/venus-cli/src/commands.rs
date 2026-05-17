use std::io::{self, Write};
use std::sync::Arc;

use venus_core::engine::QueryEngine;
use venus_core::message::Message;
use venus_core::skill::SkillRegistry;
use venus_utils::session;

use crate::render;

/// Print a line to stderr using \r\n (required for raw-mode terminal).
macro_rules! eprintlf {
    () => {
        let stderr = io::stderr();
        let mut out = stderr.lock();
        let _ = write!(out, "\r\n");
    };
    ($($arg:tt)*) => {{
        let stderr = io::stderr();
        let mut out = stderr.lock();
        let _ = write!(out, "{}\r\n", format_args!($($arg)*));
    }};
}

/// Result of handling a slash command.
pub enum CommandResult {
    /// Continue the REPL normally.
    Continue,
    /// Exit the REPL.
    Exit,
    /// Inject a message as if the user typed it (used for skill invocation).
    InjectMessage(String),
    /// Toggle vim mode in the editor.
    ToggleVim,
}

/// Handle a slash command.
pub async fn handle_command(
    input: &str,
    engine: &mut QueryEngine,
    skill_registry: Option<&Arc<SkillRegistry>>,
    plugin_registry: Option<&venus_core::plugin_registry::PluginRegistry>,
) -> CommandResult {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0];
    let _args = parts.get(1).unwrap_or(&"");

    match cmd {
        "/exit" | "/quit" | "/q" => {
            eprintlf!("Goodbye!");
            CommandResult::Exit
        }
        "/help" | "/h" => {
            print_help();
            CommandResult::Continue
        }
        "/clear" => {
            engine.messages.lock().await.clear();
            eprintlf!("  Conversation cleared.");
            CommandResult::Continue
        }
        "/cost" => {
            render::print_cost(engine);
            CommandResult::Continue
        }
        "/model" => {
            if let Some(model) = parts.get(1) {
                engine.model = model.to_string();
                eprintlf!("  Model changed to: {}", model);
            } else {
                eprintlf!("  Current model: {}", engine.model);
            }
            CommandResult::Continue
        }
        "/history" => {
            let count = engine.messages.lock().await.len();
            eprintlf!("  {} messages in conversation", count);
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
        "/config" | "/settings" => {
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
                        eprintlf!("  No saved sessions.");
                    } else {
                        eprintlf!("\r\n  Saved sessions:");
                        for (i, s) in sessions.iter().enumerate() {
                            let time = chrono::DateTime::from_timestamp(s.updated_at as i64, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_else(|| "unknown".to_string());
                            let display_name = s.name.as_deref().unwrap_or(&s.id[..8.min(s.id.len())]);
                            eprintlf!(
                                "  {:>3}. {} | {} msgs | {} | {}",
                                i + 1,
                                display_name,
                                s.message_count,
                                s.model,
                                time,
                            );
                        }
                        eprintlf!("\r\n  Use /resume <number> to resume a session.");
                    }
                }
                Err(e) => {
                    eprintlf!("  Error listing sessions: {}", e);
                }
            }
            CommandResult::Continue
        }
        "/resume" | "/continue" => {
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
                    eprintlf!("  Error listing sessions: {}", e);
                    return CommandResult::Continue;
                }
            };

            if sessions.is_empty() {
                eprintlf!("  No saved sessions.");
                return CommandResult::Continue;
            }

            let session_id = if let Some(ref a) = arg {
                if let Ok(idx) = a.parse::<usize>() {
                    if idx == 0 || idx > sessions.len() {
                        eprintlf!("  Invalid session number. Use 1-{}.", sessions.len());
                        return CommandResult::Continue;
                    }
                    sessions[idx - 1].id.clone()
                } else {
                    match sessions.iter().find(|s| s.id.starts_with(a.as_str())) {
                        Some(s) => s.id.clone(),
                        None => {
                            eprintlf!("  No session matching '{}' found.", a);
                            return CommandResult::Continue;
                        }
                    }
                }
            } else {
                eprintlf!("\r\n  Recent sessions:");
                let display_count = sessions.len().min(10);
                for (i, s) in sessions.iter().take(display_count).enumerate() {
                    let time = chrono::DateTime::from_timestamp(s.updated_at as i64, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    eprintlf!(
                        "  {:>3}. {} | {} msgs | {} | {}",
                        i + 1,
                        &s.id[..8],
                        s.message_count,
                        s.model,
                        time,
                    );
                }
                eprintlf!("\r\n  Enter number to resume (or press Enter to cancel):");
                eprint!("  > ");
                std::io::Write::flush(&mut std::io::stderr()).ok();

                let mut choice = String::new();
                if std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut choice).is_err() {
                    return CommandResult::Continue;
                }
                let choice = choice.trim();
                if choice.is_empty() {
                    eprintlf!("  Cancelled.");
                    return CommandResult::Continue;
                }
                match choice.parse::<usize>() {
                    Ok(idx) if idx >= 1 && idx <= display_count => {
                        sessions[idx - 1].id.clone()
                    }
                    _ => {
                        eprintlf!("  Invalid choice.");
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
                    eprintlf!(
                        "  Resumed session {} ({} messages)",
                        &meta.id[..8],
                        msg_count,
                    );
                }
                Err(e) => {
                    eprintlf!("  Error loading session: {}", e);
                }
            }
            CommandResult::Continue
        }
        "/commit" => {
            let diff_output = get_staged_diff(&engine.working_dir).await;
            if diff_output.is_empty() {
                eprintlf!("  No staged changes. Use `git add` first.");
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
                eprintlf!("  No changes to review.");
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
            let venus_md = engine.working_dir.join("VENUS.md");
            if venus_md.exists() {
                eprintlf!("  VENUS.md already exists.");
                return CommandResult::Continue;
            }
            CommandResult::InjectMessage(
                "Create a VENUS.md file for this project. Analyze the project structure, \
                 language, build system, and conventions. Include: project overview, \
                 build/test commands, code style, important notes.".to_string()
            )
        }
        "/memory" => {
            let arg = parts.get(1).unwrap_or(&"").trim();
            if arg.is_empty() || arg == "list" {
                match venus_utils::memory::list_memories(None, Some(&engine.working_dir)).await {
                    Ok(entries) if entries.is_empty() => eprintlf!("  No memory entries."),
                    Ok(entries) => {
                        eprintlf!("\r\n  Memory entries:");
                        for e in &entries {
                            eprintlf!(
                                "    [{}] {} ({})",
                                &e.id[..8.min(e.id.len())],
                                e.title,
                                e.memory_type
                            );
                        }
                        eprintlf!();
                    }
                    Err(e) => eprintlf!("  Error: {}", e),
                }
            } else {
                eprintlf!("  Usage: /memory [list]");
            }
            CommandResult::Continue
        }
        "/skills" => {
            if let Some(registry) = skill_registry {
                let all = registry.all();
                if all.is_empty() {
                    eprintlf!("  No skills loaded.");
                } else {
                    eprintlf!("\r\n  Loaded skills:");
                    for s in all {
                        eprintlf!("    /{} - {}", s.name, s.description);
                    }
                    eprintlf!();
                }
            } else {
                eprintlf!("  Skill registry not available.");
            }
            CommandResult::Continue
        }
        "/tasks" => {
            let tasks = engine.task_store.list();
            if tasks.is_empty() {
                eprintlf!("  No active tasks.");
            } else {
                eprintlf!("\r\n  Tasks:");
                for t in &tasks {
                    let icon = match t.status {
                        venus_core::task::TaskStatus::Pending => "○",
                        venus_core::task::TaskStatus::InProgress => "◉",
                        venus_core::task::TaskStatus::Completed => "●",
                        venus_core::task::TaskStatus::Deleted => "✗",
                    };
                    eprintlf!("    {} {} - {}", icon, t.id, t.subject);
                }
                eprintlf!();
            }
            CommandResult::Continue
        }
        "/plan" => {
            let current = engine.plan_mode.load(std::sync::atomic::Ordering::Relaxed);
            let new_val = !current;
            engine.plan_mode.store(new_val, std::sync::atomic::Ordering::Relaxed);
            eprintlf!("  Plan mode: {}", if new_val { "ON" } else { "OFF" });
            CommandResult::Continue
        }
        "/vim" => {
            CommandResult::ToggleVim
        }
        "/effort" => {
            if let Some(level) = parts.get(1) {
                match level.trim() {
                    "low" | "medium" | "high" | "max" => {
                        eprintlf!("  Effort level set to: {}", level.trim());
                    }
                    _ => eprintlf!("  Usage: /effort [low|medium|high|max]"),
                }
            } else {
                eprintlf!("  Usage: /effort [low|medium|high|max]");
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
                        eprintlf!("  Copied to clipboard.");
                    }
                }
                #[cfg(not(target_os = "macos"))]
                eprintlf!("  Clipboard not available on this platform.");
            } else {
                eprintlf!("  No assistant message to copy.");
            }
            CommandResult::Continue
        }
        "/version" => {
            eprintlf!("  Venus v{}", env!("CARGO_PKG_VERSION"));
            eprintlf!("  Model: {}", engine.model);
            eprintlf!();
            CommandResult::Continue
        }
        "/status" => {
            let uptime = chrono::Utc::now().timestamp() as u64 - engine.created_at;
            let cost = engine.cost_tracker.lock().unwrap().format_cost();
            let msg_count = engine.messages.lock().await.len();
            eprintlf!("\r\n  Session status:");
            if let Some(ref name) = engine.session_name {
                eprintlf!("    Name:       {}", name);
            }
            eprintlf!("    Uptime:     {}s", uptime);
            eprintlf!("    Messages:   {}", msg_count);
            eprintlf!("    Cost:       {}", cost);
            eprintlf!("    Model:      {}", engine.model);
            eprintlf!("    Plan mode:  {}", engine.plan_mode.load(std::sync::atomic::Ordering::Relaxed));
            eprintlf!();
            CommandResult::Continue
        }
        "/summary" => {
            if engine.messages.lock().await.len() < 4 {
                eprintlf!("  Not enough messages to summarize.");
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
                    Ok(()) => eprintlf!("  Exported {} messages to {}", values.len(), path),
                    Err(e) => eprintlf!("  Error writing {}: {}", path, e),
                },
                Err(e) => eprintlf!("  Error serializing: {}", e),
            }
            CommandResult::Continue
        }
        "/rewind" => {
            let n: usize = parts.get(1).and_then(|s| s.trim().parse().ok()).unwrap_or(1);
            let mut messages = engine.messages.lock().await;
            let total = messages.len();
            let remove = (n * 2).min(total);
            if remove == 0 {
                eprintlf!("  Nothing to rewind.");
            } else {
                messages.drain((total - remove)..);
                eprintlf!("  Rewound {} messages.", remove);
            }
            CommandResult::Continue
        }
        "/permissions" | "/allowed-tools" => {
            let subcmd = parts.get(1).map(|s| s.trim());
            match subcmd {
                Some(mode_arg) if mode_arg.starts_with("mode ") => {
                    let new_mode = mode_arg[5..].trim();
                    if ["default", "auto", "bypass"].contains(&new_mode) {
                        engine.settings = Arc::new({
                            let mut s = (*engine.settings).clone();
                            s.permission_mode = Some(new_mode.to_string());
                            s
                        });
                        eprintlf!("  Permission mode set to: {}", new_mode);
                    } else {
                        eprintlf!("  Invalid mode. Use: default, auto, bypass");
                    }
                }
                _ => {
                    let mode = engine.settings.permission_mode.as_deref().unwrap_or("default");
                    eprintlf!("\r\n  Permission mode: {} (Shift+Tab to cycle)", mode);
                    if let Some(ref allow) = engine.settings.always_allow {
                        if !allow.is_empty() {
                            eprintlf!("\r\n  Allow rules:");
                            for rule in allow {
                                eprintlf!("    ALLOW  {}:{}", rule.tool, rule.pattern.as_deref().unwrap_or("*"));
                            }
                        }
                    }
                    if let Some(ref deny) = engine.settings.always_deny {
                        if !deny.is_empty() {
                            eprintlf!("\r\n  Deny rules:");
                            for rule in deny {
                                eprintlf!("    DENY   {}:{}", rule.tool, rule.pattern.as_deref().unwrap_or("*"));
                            }
                        }
                    }
                    eprintlf!("\r\n  Usage: /permissions mode <default|auto|bypass>");
                    eprintlf!();
                }
            }
            CommandResult::Continue
        }
        "/mcp" => {
            if let Some(ref servers) = engine.settings.mcp_servers {
                if servers.is_empty() {
                    eprintlf!("  No MCP servers configured.");
                } else {
                    eprintlf!("\r\n  MCP servers:");
                    for (name, config) in servers {
                        eprintlf!("    {} - {} ({})", name, config.command, config.transport);
                    }
                    eprintlf!();
                }
            } else {
                eprintlf!("  No MCP servers configured.");
            }
            CommandResult::Continue
        }
        "/files" => {
            // List files tracked by git in the working directory
            match tokio::process::Command::new("git")
                .args(["ls-files"])
                .current_dir(&engine.working_dir)
                .output()
                .await
            {
                Ok(output) if output.status.success() => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let files: Vec<&str> = stdout.lines().collect();
                    eprintlf!("\r\n  Tracked files ({}):", files.len());
                    for f in files.iter().take(50) {
                        eprintlf!("    {}", f);
                    }
                    if files.len() > 50 {
                        eprintlf!("    ... and {} more", files.len() - 50);
                    }
                    eprintlf!();
                }
                _ => {
                    // Fallback: list files in working directory
                    eprintlf!("  Not a git repository. Listing working directory:");
                    if let Ok(entries) = std::fs::read_dir(&engine.working_dir) {
                        for entry in entries.flatten().take(30) {
                            if let Some(name) = entry.file_name().to_str() {
                                if !name.starts_with('.') {
                                    eprintlf!("    {}", name);
                                }
                            }
                        }
                    }
                    eprintlf!();
                }
            }
            CommandResult::Continue
        }
        "/keybindings" => {
            let subcmd = parts.get(1).map(|s| s.trim());
            match subcmd {
                Some("save") => {
                    let config_dir = dirs::home_dir().unwrap_or_default().join(".venus");
                    let _ = std::fs::create_dir_all(&config_dir);
                    let kb_path = config_dir.join("keybindings.json");
                    let bindings = serde_json::json!({
                        "submit": "Enter",
                        "newline": "Alt+Enter",
                        "abort": "Ctrl+C",
                        "exit": "Ctrl+D",
                        "complete": "Tab",
                        "cycle_permission": "Shift+Tab",
                        "history_up": "Up",
                        "history_down": "Down",
                    });
                    match std::fs::write(&kb_path, serde_json::to_string_pretty(&bindings).unwrap()) {
                        Ok(()) => eprintlf!("  Keybindings saved to: {}", kb_path.display()),
                        Err(e) => eprintlf!("  Error saving keybindings: {}", e),
                    }
                }
                Some("load") => {
                    let kb_path = dirs::home_dir().unwrap_or_default().join(".venus/keybindings.json");
                    match std::fs::read_to_string(&kb_path) {
                        Ok(content) => {
                            eprintlf!("  Loaded keybindings from: {}", kb_path.display());
                            if let Ok(bindings) = serde_json::from_str::<serde_json::Value>(&content) {
                                if let Some(obj) = bindings.as_object() {
                                    for (action, key) in obj {
                                        eprintlf!("    {} -> {}", action, key);
                                    }
                                }
                            }
                        }
                        Err(_) => eprintlf!("  No saved keybindings found. Use /keybindings save first."),
                    }
                }
                _ => {
                    eprintlf!("\r\n  Keyboard shortcuts:");
                    eprintlf!("    Enter           Submit input");
                    eprintlf!("    Alt+Enter       Newline in multi-line input");
                    eprintlf!("    Ctrl+C          Clear current input / abort query");
                    eprintlf!("    Ctrl+D          Exit");
                    eprintlf!("    Tab             Autocomplete slash commands");
                    eprintlf!("    Shift+Tab       Cycle permission mode");
                    eprintlf!("    Up/Down         Navigate history");
                    eprintlf!("\r\n  Keybindings management:");
                    eprintlf!("    /keybindings save   Save current keybindings to ~/.venus/keybindings.json");
                    eprintlf!("    /keybindings load   Load keybindings from config file");
                    eprintlf!("\r\n  Slash commands: /help");
                    eprintlf!();
                }
            }
            CommandResult::Continue
        }
        "/color" => {
            let color = parts.get(1).map(|s| s.trim());
            match color {
                Some("blue") | Some("green") | Some("red") | Some("yellow") | Some("cyan") | Some("magenta") | Some("white") => {
                    engine.prompt_color = color.unwrap().to_string();
                    eprintlf!("  Prompt color set to: {}", color.unwrap());
                }
                Some(other) => {
                    eprintlf!("  Unknown color: {}. Available: blue, green, red, yellow, cyan, magenta, white", other);
                }
                None => {
                    eprintlf!("  Current prompt color: {}", engine.prompt_color);
                    eprintlf!("  Usage: /color <blue|green|red|yellow|cyan|magenta|white>");
                }
            }
            CommandResult::Continue
        }
        "/theme" => {
            let theme = parts.get(1).map(|s| s.trim());
            match theme {
                Some("dark") | Some("light") | Some("auto") => {
                    engine.theme = theme.unwrap().to_string();
                    eprintlf!("  Theme set to: {}", theme.unwrap());
                }
                Some(other) => {
                    eprintlf!("  Unknown theme: {}. Available: dark, light, auto", other);
                }
                None => {
                    eprintlf!("  Current theme: {}", engine.theme);
                    eprintlf!("  Usage: /theme <dark|light|auto>");
                }
            }
            CommandResult::Continue
        }
        "/sandbox-toggle" => {
            // Toggle sandbox mode (bypass permissions on/off)
            let current = engine.settings.permission_mode.as_deref().unwrap_or("default");
            let next = if current == "bypass" { "default" } else { "bypass" };
            engine.settings = Arc::new({
                let mut s = (*engine.settings).clone();
                s.permission_mode = Some(next.to_string());
                s
            });
            eprintlf!("  Sandbox mode: {}", if next == "bypass" { "OFF (bypass)" } else { "ON (default)" });
            CommandResult::Continue
        }
        "/ps" => {
            let tasks = engine.background_runtime.list().await;
            // Also load persisted tasks from disk
            let persisted = venus_core::background::BackgroundTaskRuntime::load_from_disk()
                .await
                .unwrap_or_default();
            if tasks.is_empty() && persisted.is_empty() {
                eprintlf!("  No background tasks.");
            } else {
                if !tasks.is_empty() {
                    eprintlf!("\r\n  Active background tasks:");
                    for t in &tasks {
                        let status_icon = match t.status {
                            venus_core::background::BackgroundTaskStatus::Running => "◉",
                            venus_core::background::BackgroundTaskStatus::Completed => "●",
                            venus_core::background::BackgroundTaskStatus::Failed(_) => "✗",
                            venus_core::background::BackgroundTaskStatus::Cancelled => "◌",
                        };
                        eprintlf!("    {} {} - {}", status_icon, t.id, t.description);
                    }
                }
                // Show persisted tasks that aren't in the active list
                let active_ids: Vec<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
                let historical: Vec<_> = persisted.iter()
                    .filter(|t| !active_ids.contains(&t.id.as_str()))
                    .collect();
                if !historical.is_empty() {
                    eprintlf!("\r\n  Historical tasks:");
                    for t in historical.iter().take(10) {
                        let status_icon = match t.status {
                            venus_core::background::BackgroundTaskStatus::Running => "◉",
                            venus_core::background::BackgroundTaskStatus::Completed => "●",
                            venus_core::background::BackgroundTaskStatus::Failed(_) => "✗",
                            venus_core::background::BackgroundTaskStatus::Cancelled => "◌",
                        };
                        eprintlf!("    {} {} - {}", status_icon, t.id, t.description);
                    }
                }
                eprintlf!("\r\n  Use /attach <id> to view output, /kill <id> to stop.");
                eprintlf!();
            }
            CommandResult::Continue
        }
        "/attach" => {
            if let Some(id) = parts.get(1).map(|s| s.trim()) {
                // Try in-memory first
                match engine.background_runtime.read_output(id).await {
                    Ok((info, output)) => {
                        eprintlf!("\r\n  Task: {} - {}", info.id, info.description);
                        eprintlf!("  Status: {:?}", info.status);
                        eprintlf!("  Output:");
                        for line in output.lines() {
                            eprintlf!("    {}", line);
                        }
                        eprintlf!();
                    }
                    Err(_) => {
                        // Fall back to persisted tasks on disk
                        match venus_core::background::BackgroundTaskRuntime::load_from_disk().await {
                            Ok(tasks) => {
                                if let Some(info) = tasks.iter().find(|t| t.id == id || id.starts_with(&t.id)) {
                                    eprintlf!("\r\n  Task: {} - {}", info.id, info.description);
                                    eprintlf!("  Status: {:?}", info.status);
                                    if let Some(ref output) = info.output {
                                        eprintlf!("  Output:");
                                        for line in output.lines() {
                                            eprintlf!("    {}", line);
                                        }
                                    } else {
                                        eprintlf!("  No output recorded.");
                                    }
                                    eprintlf!();
                                } else {
                                    eprintlf!("  Task '{}' not found.", id);
                                }
                            }
                            Err(e) => eprintlf!("  Error loading tasks: {}", e),
                        }
                    }
                }
            } else {
                eprintlf!("  Usage: /attach <task-id>");
            }
            CommandResult::Continue
        }
        "/kill" => {
            if let Some(id) = parts.get(1).map(|s| s.trim()) {
                match engine.background_runtime.cancel(id).await {
                    Ok(true) => eprintlf!("  Task {} cancelled.", id),
                    Ok(false) => eprintlf!("  Task {} not found or already finished.", id),
                    Err(e) => eprintlf!("  Error: {}", e),
                }
            } else {
                eprintlf!("  Usage: /kill <task-id>");
            }
            CommandResult::Continue
        }
        "/stats" => {
            let uptime = chrono::Utc::now().timestamp() as u64 - engine.created_at;
            let msg_count = engine.messages.lock().await.len();
            let cost = engine.cost_tracker.lock().unwrap().format_cost();
            let tokens = engine.cost_tracker.lock().unwrap().format_tokens();
            let tool_count = engine.tools.all().len();
            eprintlf!("\r\n  Statistics:");
            eprintlf!("    Uptime:        {}s", uptime);
            eprintlf!("    Messages:      {}", msg_count);
            eprintlf!("    Tools:         {}", tool_count);
            eprintlf!("    Cost:          {}", cost);
            eprintlf!("    Tokens:        {}", tokens);
            eprintlf!("    Model:         {}", engine.model);
            eprintlf!("    Max turns:     {}", engine.max_turns);
            if let Some(budget) = engine.budget_usd {
                eprintlf!("    Budget:        ${:.2}", budget);
            }
            eprintlf!();
            CommandResult::Continue
        }
        "/agents" => {
            let tasks = engine.task_store.list();
            let bg_tasks = engine.background_runtime.list().await;
            if tasks.is_empty() && bg_tasks.is_empty() {
                eprintlf!("  No active agents or background tasks.");
            } else {
                if !tasks.is_empty() {
                    eprintlf!("\r\n  Tasks:");
                    for t in &tasks {
                        let status_icon = match t.status {
                            venus_core::task::TaskStatus::Pending => "○",
                            venus_core::task::TaskStatus::InProgress => "◉",
                            venus_core::task::TaskStatus::Completed => "●",
                            venus_core::task::TaskStatus::Deleted => "◌",
                        };
                        eprintlf!("    {} {} - {}", status_icon, t.id, t.subject);
                    }
                }
                if !bg_tasks.is_empty() {
                    eprintlf!("\r\n  Background tasks:");
                    for t in &bg_tasks {
                        eprintlf!("    {} - {}", t.id, t.description);
                    }
                }
                eprintlf!();
            }
            CommandResult::Continue
        }
        "/add-dir" => {
            if let Some(dir) = parts.get(1).map(|s| s.trim()) {
                let path = std::path::PathBuf::from(dir);
                if path.is_dir() {
                    engine.additional_working_dirs.push(path.clone());
                    eprintlf!("  Added directory: {}", path.display());
                } else {
                    eprintlf!("  Not a directory: {}", dir);
                }
            } else if engine.additional_working_dirs.is_empty() {
                eprintlf!("  No additional directories. Usage: /add-dir <path>");
            } else {
                eprintlf!("  Additional directories:");
                for dir in &engine.additional_working_dirs {
                    eprintlf!("    {}", dir.display());
                }
            }
            CommandResult::Continue
        }
        "/output-style" => {
            let style = parts.get(1).map(|s| s.trim());
            match style {
                Some("default") | Some("explanatory") | Some("learning") => {
                    let prompt = match style.unwrap() {
                        "explanatory" => "\n\n# Output Style: Explanatory\nExplain your implementation choices. After each code change, add an 'Insight' block explaining why you made those specific decisions. Help the user understand the reasoning behind the approach.",
                        "learning" => "\n\n# Output Style: Learning\nTeach as you go. After explaining a concept, pause and ask the user to try writing the code themselves before you provide the solution. Use a hands-on teaching approach.",
                        _ => "",
                    };
                    if !prompt.is_empty() {
                        engine.system_prompt.push_str(prompt);
                        eprintlf!("  Output style set to: {}", style.unwrap());
                    }
                }
                Some(other) => {
                    eprintlf!("  Unknown style: {}. Use: default, explanatory, learning", other);
                }
                None => {
                    eprintlf!("  Usage: /output-style <default|explanatory|learning>");
                    eprintlf!("  Current styles inject additional instructions into the system prompt.");
                }
            }
            CommandResult::Continue
        }
        "/branch" => {
            // Show git branch info
            match tokio::process::Command::new("git")
                .args(["branch", "-v", "--no-color"])
                .current_dir(&engine.working_dir)
                .output()
                .await
            {
                Ok(output) if output.status.success() => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    eprintlf!("\r\n  Branches:");
                    for line in stdout.lines() {
                        eprintlf!("    {}", line);
                    }
                    eprintlf!();
                }
                _ => eprintlf!("  Not a git repository or git not available."),
            }
            CommandResult::Continue
        }
        "/btw" => {
            // Quick note - inject as context message
            let note = parts.get(1).map(|s| s.trim()).unwrap_or("");
            if note.is_empty() {
                eprintlf!("  Usage: /btw <note> - Add a quick note to the conversation");
            } else {
                eprintlf!("  Noted: {}", note);
            }
            CommandResult::Continue
        }
        "/tag" => {
            // Show or create git tags
            match tokio::process::Command::new("git")
                .args(["tag", "-l", "--sort=-creatordate"])
                .current_dir(&engine.working_dir)
                .output()
                .await
            {
                Ok(output) if output.status.success() => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let tags: Vec<&str> = stdout.lines().collect();
                    if tags.is_empty() {
                        eprintlf!("  No tags found.");
                    } else {
                        eprintlf!("\r\n  Tags (recent first):");
                        for t in tags.iter().take(20) {
                            eprintlf!("    {}", t);
                        }
                        if tags.len() > 20 {
                            eprintlf!("    ... and {} more", tags.len() - 20);
                        }
                    }
                    eprintlf!();
                }
                _ => eprintlf!("  Not a git repository or git not available."),
            }
            CommandResult::Continue
        }
        "/fast" => {
            // Toggle between a fast model and the configured model
            let fast_model = "claude-haiku-4-5-20251001";
            if engine.model == fast_model {
                // Restore original model from settings
                let original = engine.settings.effective_model().to_string();
                engine.model = original.clone();
                eprintlf!("  Switched to full model: {}", original);
            } else {
                engine.model = fast_model.to_string();
                eprintlf!("  Switched to fast model: {}", fast_model);
            }
            CommandResult::Continue
        }
        "/rename" | "/rename-session" => {
            let name = parts.get(1).map(|s| s.trim().to_string());
            if let Some(name) = name {
                engine.session_name = Some(name.clone());
                eprintlf!("  Session renamed to: {}", name);
            } else {
                engine.session_name = None;
                eprintlf!("  Session name cleared.");
            }
            CommandResult::Continue
        }
        "/hooks" => {
            if let Some(ref hooks) = engine.settings.hooks {
                if hooks.entries.is_empty() {
                    eprintlf!("  No hooks configured.");
                } else {
                    eprintlf!("\r\n  Configured hooks:");
                    for (event, hook_list) in &hooks.entries {
                        for entry in hook_list {
                            let matcher = entry.matcher.as_deref().unwrap_or("*");
                            for hook in &entry.hooks {
                                eprintlf!(
                                    "    {} ({}) -> {}",
                                    event, matcher, hook.command
                                );
                            }
                        }
                    }
                    eprintlf!();
                }
            } else {
                eprintlf!("  No hooks configured.");
            }
            CommandResult::Continue
        }
        "/delete-session" => {
            let arg = parts.get(1).map(|s| s.trim().to_string());
            if let Some(arg) = arg {
                // Try to parse as number (session list index) or use as ID prefix
                let rt = tokio::runtime::Handle::current();
                let sessions = std::thread::spawn({
                    let rt = rt.clone();
                    move || rt.block_on(session::list_sessions())
                })
                .join()
                .unwrap_or_else(|_| Ok(Vec::new()));

                match sessions {
                    Ok(sessions) => {
                        let session_id = if let Ok(idx) = arg.parse::<usize>() {
                            if idx == 0 || idx > sessions.len() {
                                eprintlf!("  Invalid session number. Use 1-{}.", sessions.len());
                                return CommandResult::Continue;
                            }
                            sessions[idx - 1].id.clone()
                        } else {
                            match sessions.iter().find(|s| s.id.starts_with(&arg)) {
                                Some(s) => s.id.clone(),
                                None => {
                                    eprintlf!("  No session found matching '{}'.", arg);
                                    return CommandResult::Continue;
                                }
                            }
                        };

                        match std::thread::spawn({
                            let rt = rt.clone();
                            let sid = session_id.clone();
                            move || rt.block_on(session::delete_session(&sid))
                        })
                        .join()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("thread panic")))
                        {
                            Ok(()) => eprintlf!("  Deleted session {}.", &session_id[..8.min(session_id.len())]),
                            Err(e) => eprintlf!("  Error deleting session: {}", e),
                        }
                    }
                    Err(e) => eprintlf!("  Error listing sessions: {}", e),
                }
            } else {
                eprintlf!("  Usage: /delete-session <number|id-prefix>");
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
                eprintlf!("  Invoking skill: {}", skill.name);
                let content = if let Some(args) = parts.get(1) {
                    format!("{}\n\nArguments: {}", skill.content, args)
                } else {
                    skill.content.clone()
                };
                return CommandResult::InjectMessage(content);
            }

            // Check plugin commands
            if let Some(registry) = plugin_registry {
                let cmd_name = &cmd[1..]; // strip leading /
                for plugin in registry.all_plugins() {
                    for cmd_def in &plugin.manifest.commands {
                        if cmd_def.name == cmd_name {
                            if let Some(ref prompt) = cmd_def.prompt {
                                eprintlf!("  Invoking plugin command: {}", cmd_def.name);
                                let content = if let Some(args) = parts.get(1) {
                                    format!("{}\n\nArguments: {}", prompt, args)
                                } else {
                                    prompt.clone()
                                };
                                return CommandResult::InjectMessage(content);
                            }
                        }
                    }
                }
            }

            eprintlf!("  Unknown command: {}", cmd);
            eprintlf!("  Type /help for available commands");
            CommandResult::Continue
        }
    }
}

/// Run `git diff` and `git diff --staged` in the working directory and display output.
async fn handle_diff(engine: &QueryEngine) {
    eprintlf!();

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
                eprintlf!("  No unstaged changes.");
            } else {
                eprintlf!("  \x1b[1mUnstaged changes:\x1b[0m");
                for line in stdout.lines() {
                    eprintlf!("  {}", line);
                }
            }
        }
        Err(e) => {
            eprintlf!("  \x1b[31mFailed to run git diff: {}\x1b[0m", e);
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
                eprintlf!("  No staged changes.");
            } else {
                eprintlf!("\r\n  \x1b[1mStaged changes:\x1b[0m");
                for line in stdout.lines() {
                    eprintlf!("  {}", line);
                }
            }
        }
        Err(e) => {
            eprintlf!("  \x1b[31mFailed to run git diff --staged: {}\x1b[0m", e);
        }
    }

    eprintlf!();
}

/// Compact conversation using AI summarization, with fallback to naive truncation.
async fn handle_compact(engine: &QueryEngine) {
    let total = engine.messages.lock().await.len();

    if total <= 4 {
        eprintlf!("\r\n  Conversation has {} messages, nothing to compact.", total);
        return;
    }

    eprintlf!("\r\n  Compacting conversation ({} messages)...", total);

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
            eprintlf!(
                "  Compacted: {} -> {} messages (~{} tokens saved)",
                result.messages_before, result.messages_after, result.tokens_saved_estimate,
            );
        }
        Err(e) => {
            eprintlf!("  AI summarization failed: {}", e);
            // Fallback to naive compaction
            let keep = 10.min(total);
            let removed = total - keep;
            messages.drain(..removed);
            eprintlf!(
                "  Fell back to keeping last {} messages (removed {}).",
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

    let provider_name = engine.settings.active_provider.as_deref().unwrap_or("(none)");
    let provider_type = engine.settings.provider_type();

    eprintlf!("\r\n  \x1b[1mConfiguration:\x1b[0m");
    eprintlf!("    Provider:          {} ({})", provider_name, provider_type);
    eprintlf!("    Model:             {}", engine.model);
    eprintlf!("    Base URL:          {}", engine.base_url);
    eprintlf!("    Working dir:       {}", engine.working_dir.display());
    eprintlf!("    Permission mode:   {}", permission_mode);
    eprintlf!("    Prompt color:      {}", engine.prompt_color);
    eprintlf!("    Theme:             {}", engine.theme);
    eprintlf!("    Max tokens:        {}", engine.max_tokens);
    eprintlf!("    Max turns:         {}", engine.max_turns);
    if let Some(budget) = engine.budget_usd {
        eprintlf!("    Budget:            ${:.2}", budget);
    }
    if let Some(ref thinking) = engine.settings.thinking {
        eprintlf!("    Thinking:          {}", thinking.mode.as_deref().unwrap_or("default"));
    }
    if let Some(ref allow) = engine.settings.always_allow {
        eprintlf!("    Allow rules:       {}", allow.len());
    }
    if let Some(ref deny) = engine.settings.always_deny {
        eprintlf!("    Deny rules:        {}", deny.len());
    }
    if let Some(ref mcp) = engine.settings.mcp_servers {
        eprintlf!("    MCP servers:       {}", mcp.len());
    }
    eprintlf!(
        "    Token usage:       {} input, {} output",
        total_usage.input_tokens + total_usage.cache_read_tokens,
        total_usage.output_tokens
    );
}

/// Run environment diagnostics, checking for required tools and config.
async fn handle_doctor(engine: &QueryEngine) {
    eprintlf!("\r\n  \x1b[1mEnvironment diagnostics:\x1b[0m");

    // Check git
    let git_ok = check_command("git", &["--version"], &engine.working_dir).await;
    print_check("git", git_ok);

    // Check ripgrep
    let rg_ok = check_command("rg", &["--version"], &engine.working_dir).await;
    print_check("rg (ripgrep)", rg_ok);

    // Check API credentials in config
    let has_credential = engine.settings.resolve_auth().is_some();
    print_check("API credential (in config)", has_credential);

    // Check ~/.venus/config.toml
    let config_path = dirs_path("config.toml");
    let config_exists = config_path
        .map(|p| p.exists())
        .unwrap_or(false);
    print_check("~/.venus/config.toml", config_exists);

    eprintlf!();
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

    eprintlf!("\r\n  \x1b[1mContext info:\x1b[0m");
    eprintlf!("    Model:               {}", engine.model);
    eprintlf!("    Context window:      {} tokens", window);
    eprintlf!("    Auto-compact at:     {} tokens", threshold);
    eprintlf!("    Messages:            {}", analysis.message_count);
    eprintlf!("    Turns:               {}", analysis.turn_count);
    eprintlf!("    Usage:               ~{}% of context window", usage_pct);
    eprintlf!();
    eprintlf!("    \x1b[1mToken breakdown (estimated):\x1b[0m");
    eprintlf!("      System prompt:     {}", analysis.system_prompt_tokens);
    eprintlf!("      User text:         {}", analysis.user_text_tokens);
    eprintlf!("      Assistant text:    {}", analysis.assistant_text_tokens);
    eprintlf!(
        "      Tool requests:     {}",
        analysis.tool_request_tokens.values().sum::<u64>()
    );
    eprintlf!(
        "      Tool results:      {}",
        analysis.tool_result_tokens.values().sum::<u64>()
    );
    eprintlf!("      Thinking:          {}", analysis.thinking_tokens);
    eprintlf!("      \x1b[1mTotal:             {}\x1b[0m", analysis.total_tokens);

    // Show per-tool breakdown if there are tool results
    if !analysis.tool_result_tokens.is_empty() {
        eprintlf!();
        eprintlf!("    \x1b[1mTool result tokens:\x1b[0m");
        let mut sorted: Vec<_> = analysis.tool_result_tokens.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (name, tokens) in sorted {
            eprintlf!("      {:<20} {}", name, tokens);
        }
    }

    if !analysis.duplicate_file_reads.is_empty() {
        eprintlf!();
        eprintlf!("    \x1b[33mDuplicate file reads:\x1b[0m");
        for (path, count) in &analysis.duplicate_file_reads {
            eprintlf!("      {} ({}x)", path, count);
        }
    }

    eprintlf!();
}

/// Display detailed per-model token breakdown.
fn handle_tokens(engine: &QueryEngine) {
    eprintlf!("\r\n  \x1b[1mToken breakdown:\x1b[0m");

    let tracker = engine.cost_tracker.lock().unwrap();
    if tracker.usage_by_model.is_empty() {
        eprintlf!("    No token usage recorded yet.");
        return;
    }

    for (model, usage) in &tracker.usage_by_model {
        eprintlf!("    \x1b[33m{}\x1b[0m", model);
        eprintlf!("      Input tokens:          {}", usage.input_tokens);
        eprintlf!("      Output tokens:         {}", usage.output_tokens);
        eprintlf!("      Cache read tokens:     {}", usage.cache_read_tokens);
        eprintlf!("      Cache creation tokens: {}", usage.cache_creation_tokens);
    }

    let cost = tracker.format_cost();
    eprintlf!("\r\n    Total cost: {}", cost);
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
    eprintlf!("    [{}] {}", icon, label);
}

/// Resolve ~/.claude/<filename> path.
fn dirs_path(filename: &str) -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".venus").join(filename))
}

/// List installed plugins from standard directories.
async fn handle_plugins() {
    let plugin_dirs = vec![
        dirs::home_dir()
            .unwrap_or_default()
            .join(".venus")
            .join("plugins"),
        std::env::current_dir()
            .unwrap_or_default()
            .join(".venus")
            .join("plugins"),
    ];

    let mut registry = venus_core::plugin_registry::PluginRegistry::new();
    if let Err(e) = registry.load_all(&plugin_dirs).await {
        eprintlf!("  Error loading plugins: {}", e);
        return;
    }

    let plugins = registry.all_plugins();
    if plugins.is_empty() {
        let stderr = io::stderr();
        let mut out = stderr.lock();
        let _ = write!(out, "\r\n  No plugins installed.\r\n\r\n  Place plugins in ~/.venus/plugins/ or ./.venus/plugins/.\r\n  Each plugin directory must contain a plugin.json manifest.\r\n\r\n");
        return;
    }

    eprintlf!("\r\n  \x1b[1mInstalled plugins:\x1b[0m");
    for plugin in plugins {
        let desc = plugin
            .manifest
            .description
            .as_deref()
            .unwrap_or("(no description)");
        eprintlf!(
            "    \x1b[33m{}\x1b[0m v{} - {}",
            plugin.manifest.name, plugin.manifest.version, desc
        );
        if !plugin.manifest.tools.is_empty() {
            let tool_names: Vec<&str> =
                plugin.manifest.tools.iter().map(|t| t.name.as_str()).collect();
            eprintlf!("      Tools: {}", tool_names.join(", "));
        }
        if !plugin.manifest.mcp_servers.is_empty() {
            let server_names: Vec<&str> = plugin.manifest.mcp_servers.keys().map(|s| s.as_str()).collect();
            eprintlf!("      MCP servers: {}", server_names.join(", "));
        }
        if !plugin.manifest.commands.is_empty() {
            let cmd_names: Vec<&str> =
                plugin.manifest.commands.iter().map(|c| c.name.as_str()).collect();
            eprintlf!("      Commands: {}", cmd_names.join(", "));
        }
    }
    eprintlf!();
}

fn print_help() {
    let stderr = io::stderr();
    let mut out = stderr.lock();
    let help = "\
\r\n  Available commands:\
\r\n    /help, /h       Show this help\
\r\n    /exit, /quit    Exit the REPL\
\r\n    /clear          Clear conversation history\
\r\n    /cost           Show token usage and cost\
\r\n    /model [name]   Show or change model\
\r\n    /history        Show conversation message count\
\r\n    /diff           Show git diff (staged + unstaged)\
\r\n    /compact        Compact conversation with AI summarization\
\r\n    /config         Show current configuration\
\r\n    /doctor         Run environment diagnostics\
\r\n    /context        Show context info\
\r\n    /tokens         Show detailed token breakdown\
\r\n    /plugin         List installed plugins\
\r\n    /sessions       List all saved sessions\
\r\n    /resume [n|id]  Resume a previous session\
\r\n    /commit         Generate conventional commit from staged changes\
\r\n    /review         Review code changes for issues\
\r\n    /init           Create VENUS.md for this project\
\r\n    /memory [list]  List memory entries\
\r\n    /skills         List loaded skills\
\r\n    /tasks          List active tasks\
\r\n    /plan           Toggle plan mode\
\r\n    /vim            Toggle vim mode (pending)\
\r\n    /effort [level] Set effort level (low/medium/high/max)\
\r\n    /copy           Copy last assistant message to clipboard\
\r\n    /version        Show version and model info\
\r\n    /status         Show session status\
\r\n    /summary        Summarize conversation\
\r\n    /export [path]  Export conversation to JSON\
\r\n    /rewind [n]     Rewind n message pairs\
\r\n    /permissions    Show permission rules\
\r\n    /mcp            Show MCP server config\
\r\n    /files          List tracked project files\
\r\n    /keybindings    Show keyboard shortcuts\
\r\n    /color <name>   Set prompt color\
\r\n    /theme <name>   Set terminal theme\
\r\n    /sandbox-toggle Toggle sandbox mode\
\r\n    /stats          Show statistics\
\r\n    /agents         List active agents/tasks\
\r\n    /ps             List background tasks\
\r\n    /attach <id>    View background task output\
\r\n    /kill <id>      Stop background task\
\r\n    /output-style   Set output style (default/explanatory/learning)\
\r\n    /branch         Show git branches\
\r\n    /btw <note>     Add a quick note\
\r\n    /tag            Show git tags\
\r\n    /add-dir [path] Add working directory context\
\r\n    /fast           Toggle fast model mode\
\r\n    /rename [name]  Rename current session\
\r\n    /hooks          Show configured hooks\
\r\n    /delete-session Delete a saved session\
\r\n\
\r\n  Keyboard:\
\r\n    Ctrl+C          Abort current query\
\r\n    Ctrl+D          Exit\
\r\n";
    let _ = write!(out, "{}", help);
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
