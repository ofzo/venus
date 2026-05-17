mod app;
mod commands;
mod event;
mod input_state;
mod markdown_tui;
mod render;
mod tui;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

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

use venus_core::background::BackgroundTaskRuntime;
use venus_core::engine::QueryEngine;
use venus_core::hooks::HookRunner;
use venus_core::skill::SkillRegistry;
use venus_core::task::TaskStore;
use venus_core::tool_registry::ToolRegistry;
use venus_permissions::interactive::InteractivePermissionHandler;
use venus_permissions::tui_handler::TuiPermissionHandler;
use venus_utils::config::Settings;
use render::OutputFormat;

#[derive(Parser, Debug)]
#[command(name = "venus", about = "Venus - AI coding assistant")]
struct Cli {
    /// Model to use
    #[arg(short, long)]
    model: Option<String>,

    /// API key (for quick setup, prefer config file)
    #[arg(short = 'k', long)]
    api_key: Option<String>,

    /// Non-interactive mode: run a single prompt
    #[arg(short, long)]
    prompt: Option<String>,

    /// Additional system prompt
    #[arg(long)]
    system: Option<String>,

    /// Working directory
    #[arg(short = 'd', long)]
    working_dir: Option<PathBuf>,

    /// Resume a previous session by ID (or prefix)
    #[arg(long)]
    resume: Option<String>,

    /// Continue the most recent session
    #[arg(short = 'c', long)]
    r#continue: bool,

    /// Session display name
    #[arg(short = 'n', long)]
    name: Option<String>,

    /// Thinking mode (enabled/adaptive/disabled)
    #[arg(long)]
    thinking: Option<String>,

    /// Effort level (low/medium/high/max)
    #[arg(long)]
    effort: Option<String>,

    /// Maximum agentic turns
    #[arg(long)]
    max_turns: Option<u32>,

    /// Output format
    #[arg(long, value_enum, default_value = "text")]
    output_format: OutputFormat,

    /// Verbose/debug mode
    #[arg(short = 'v', long)]
    verbose: bool,

    /// Skip all permission checks
    #[arg(long)]
    dangerously_skip_permissions: bool,

    /// Permission mode (default/auto/bypass)
    #[arg(long)]
    permission_mode: Option<String>,

    /// Allowed tools (comma-separated)
    #[arg(long, value_delimiter = ',')]
    allowed_tools: Option<Vec<String>>,

    /// Disallowed tools (comma-separated)
    #[arg(long, value_delimiter = ',')]
    disallowed_tools: Option<Vec<String>>,

    /// Append to system prompt
    #[arg(long)]
    append_system_prompt: Option<String>,

    /// Read system prompt from file
    #[arg(long)]
    system_prompt_file: Option<PathBuf>,

    /// Maximum budget in USD
    #[arg(long)]
    max_budget_usd: Option<f64>,

    /// MCP config file path
    #[arg(long)]
    mcp_config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging with verbose override
    let log_level = if cli.verbose { "debug" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
        )
        .with_target(false)
        .init();

    let working_dir = cli
        .working_dir
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Load settings with multi-level merging (global -> project -> env vars)
    let mut settings =
        Settings::load_with_project(Some(&working_dir)).context("failed to load settings")?;

    // CLI overrides (highest priority)
    if let Some(model) = cli.model {
        settings.model = Some(model);
    }
    if let Some(key) = cli.api_key {
        // Set API key on the active provider (create default anthropic provider if needed)
        use std::collections::HashMap;
        let providers = settings.provider.get_or_insert_with(HashMap::new);
        let provider_name = settings.active_provider.as_deref().unwrap_or("anthropic");
        providers
            .entry(provider_name.to_string())
            .or_insert_with(|| venus_utils::config::ProviderConfig {
                provider_type: "anthropic".to_string(),
                api_key: None,
                auth_token: None,
                base_url: None,
                default_model: None,
                max_tokens: None,
                api_version: None,
            })
            .api_key = Some(key);
        if settings.active_provider.is_none() {
            settings.active_provider = Some("anthropic".to_string());
        }
    }

    // --thinking: override settings.thinking
    if let Some(ref thinking_mode) = cli.thinking {
        use venus_utils::config::ThinkingConfig;
        settings.thinking = Some(ThinkingConfig {
            mode: Some(thinking_mode.clone()),
            budget_tokens: settings
                .thinking
                .as_ref()
                .and_then(|t| t.budget_tokens),
        });
    }

    // --effort: map effort level to thinking budget
    if let Some(ref effort) = cli.effort {
        use venus_utils::config::ThinkingConfig;
        let budget = match effort.as_str() {
            "low" => Some(1024),
            "medium" => Some(4096),
            "high" => Some(10000),
            "max" => Some(32000),
            _ => None,
        };
        let existing_mode = settings
            .thinking
            .as_ref()
            .and_then(|t| t.mode.clone());
        settings.thinking = Some(ThinkingConfig {
            mode: existing_mode.or_else(|| Some("enabled".to_string())),
            budget_tokens: budget,
        });
    }

    // --permission-mode: override settings.permission_mode
    if let Some(ref mode) = cli.permission_mode {
        settings.permission_mode = Some(mode.clone());
    }

    // --dangerously-skip-permissions: set permission_mode to bypass
    if cli.dangerously_skip_permissions {
        settings.permission_mode = Some("bypass".into());
    }

    // --allowed-tools / --disallowed-tools: override settings
    if cli.allowed_tools.is_some() {
        settings.allowed_tools = cli.allowed_tools;
    }
    if cli.disallowed_tools.is_some() {
        settings.disallowed_tools = cli.disallowed_tools;
    }

    // --mcp-config: read and merge MCP config
    if let Some(ref mcp_path) = cli.mcp_config {
        let mcp_content = std::fs::read_to_string(mcp_path)
            .with_context(|| format!("failed to read MCP config: {}", mcp_path.display()))?;
        let mcp_servers: std::collections::HashMap<String, venus_utils::config::McpServerConfig> =
            serde_json::from_str(&mcp_content)
                .with_context(|| format!("failed to parse MCP config: {}", mcp_path.display()))?;
        settings.mcp_servers = Some(mcp_servers);
    }

    let settings = Arc::new(settings);

    // Load skills from ~/.claude/skills/ and <project>/.claude/skills/
    let skill_dirs = vec![
        dirs::home_dir()
            .unwrap_or_default()
            .join(".venus")
            .join("skills"),
        working_dir.join(".venus").join("skills"),
    ];
    let skill_registry = Arc::new(
        SkillRegistry::load_from_dirs(&skill_dirs)
            .await
            .unwrap_or_else(|_| SkillRegistry::new()),
    );

    // Build tool list, including SkillTool with the loaded registry
    let mut all_tool_list = venus_tools::all_tools();
    all_tool_list.push(Box::new(venus_tools::skill::SkillTool::new(
        skill_registry.clone(),
    )));

    // Load plugins from ~/.claude/plugins/ and <project>/.claude/plugins/
    let plugin_dirs = vec![
        dirs::home_dir()
            .unwrap_or_default()
            .join(".venus")
            .join("plugins"),
        working_dir.join(".venus").join("plugins"),
    ];
    let mut plugin_registry = venus_core::plugin_registry::PluginRegistry::new();
    if let Err(e) = plugin_registry.load_all(&plugin_dirs).await {
        eprintlf!("Warning: failed to load plugins: {}", e);
    }
    // Add plugin tools to the tool list
    for plugin in plugin_registry.all_plugins() {
        for tool_def in &plugin.manifest.tools {
            all_tool_list.push(Box::new(venus_tools::plugin_tool::PluginTool {
                tool_def: tool_def.clone(),
                base_dir: plugin.base_dir.clone(),
            }));
        }
    }

    // Add MCP tools if configured
    let _mcp_manager = if let Some(ref mcp_configs) = settings.mcp_servers {
        if !mcp_configs.is_empty() {
            match venus_mcp::McpManager::start_all(mcp_configs).await {
                Ok(manager) => {
                    all_tool_list.extend(manager.all_tools());
                    Some(manager)
                }
                Err(e) => {
                    eprintlf!("Warning: failed to start MCP servers: {}", e);
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    let tools = Arc::new(ToolRegistry::new_filtered(
        all_tool_list,
        settings.allowed_tools.as_deref(),
        settings.disallowed_tools.as_deref(),
    ));
    let task_store = Arc::new(TaskStore::new());
    let background_runtime = Arc::new(BackgroundTaskRuntime::new());
    let hook_runner = Arc::new(HookRunner::new(
        settings.hooks.clone(),
        String::new(),
        working_dir.clone(),
    ));

    // Create permission handler based on mode
    let is_interactive = cli.prompt.is_none();
    let (perm_tx, perm_rx) = if is_interactive {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<venus_permissions::tui_handler::PermissionRequest>();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let permissions: Arc<dyn venus_core::tool::PermissionHandler> = if let Some(tx) = perm_tx {
        Arc::new(TuiPermissionHandler::new(settings.clone(), tx))
    } else {
        Arc::new(InteractivePermissionHandler::new(settings.clone()))
    };

    let mut engine =
        QueryEngine::new(settings.clone(), tools, permissions, working_dir.clone(), task_store, background_runtime, hook_runner).await?;

    // CLI flag overrides (applied after settings-based engine init)
    if let Some(ref name) = cli.name {
        engine.session_name = Some(name.clone());
    }
    if let Some(max_turns) = cli.max_turns {
        engine.max_turns = max_turns;
    }
    if let Some(budget) = cli.max_budget_usd {
        engine.budget_usd = Some(budget);
    }
    if let Some(ref append) = cli.append_system_prompt {
        engine.system_prompt.push_str("\n\n");
        engine.system_prompt.push_str(append);
    }
    if let Some(ref prompt_file) = cli.system_prompt_file {
        let content = std::fs::read_to_string(prompt_file)
            .with_context(|| format!("failed to read system prompt file: {}", prompt_file.display()))?;
        engine.system_prompt = content;
    }

    // Resume session if requested
    if let Some(resume_id) = cli.resume {
        match venus_utils::session::load_session(&resume_id).await {
            Ok((meta, msg_values)) => {
                let messages: Vec<venus_core::message::Message> = msg_values
                    .iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect();
                let msg_count = messages.len();
                *engine.messages.lock().await = messages;
                engine.session_id = meta.id.clone();
                engine.created_at = meta.created_at;
                eprintlf!(
                    "Resumed session {} ({} messages)",
                    &meta.id[..8.min(meta.id.len())],
                    msg_count,
                );
            }
            Err(e) => {
                match try_resume_by_prefix(&resume_id).await {
                    Ok(Some((meta, msg_values))) => {
                        let messages: Vec<venus_core::message::Message> = msg_values
                            .iter()
                            .filter_map(|v| serde_json::from_value(v.clone()).ok())
                            .collect();
                        let msg_count = messages.len();
                        *engine.messages.lock().await = messages;
                        engine.session_id = meta.id.clone();
                        engine.created_at = meta.created_at;
                        eprintlf!(
                            "Resumed session {} ({} messages)",
                            &meta.id[..8.min(meta.id.len())],
                            msg_count,
                        );
                    }
                    _ => {
                        eprintlf!("Warning: could not resume session '{}': {}", resume_id, e);
                    }
                }
            }
        }
    }

    // --continue: load the most recent session
    if cli.r#continue {
        match venus_utils::session::list_sessions().await {
            Ok(sessions) => {
                if let Some(latest) = sessions.first() {
                    match venus_utils::session::load_session(&latest.id).await {
                        Ok((meta, msg_values)) => {
                            let messages: Vec<venus_core::message::Message> = msg_values
                                .iter()
                                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                                .collect();
                            let msg_count = messages.len();
                            *engine.messages.lock().await = messages;
                            engine.session_id = meta.id.clone();
                            engine.created_at = meta.created_at;
                            eprintlf!(
                                "Continued session {} ({} messages)",
                                &meta.id[..8.min(meta.id.len())],
                                msg_count,
                            );
                        }
                        Err(e) => {
                            eprintlf!("Warning: failed to load latest session: {}", e);
                        }
                    }
                } else {
                    eprintlf!("No saved sessions to continue.");
                }
            }
            Err(e) => {
                eprintlf!("Warning: failed to list sessions: {}", e);
            }
        }
    }

    // Non-interactive mode
    if let Some(prompt) = cli.prompt {
        let content = vec![venus_core::message::ContentBlock::text(&prompt)];
        let mut rx = engine.submit_message(content).await?;

        // Drain stream events
        while let Some(event) = rx.recv().await {
            match event {
                venus_core::stream::StreamEvent::TextDelta(text) => {
                    eprint!("{}", text);
                }
                venus_core::stream::StreamEvent::Error(err) => {
                    eprintln!("\nError: {}", err);
                }
                venus_core::stream::StreamEvent::Usage(usage) => {
                    let total = usage.input_tokens + usage.cache_read_tokens + usage.output_tokens;
                    eprintln!("\n\ntokens: {} (in:{} out:{})", total, usage.input_tokens + usage.cache_read_tokens, usage.output_tokens);
                }
                venus_core::stream::StreamEvent::MessageComplete(_) => {
                    eprintln!();
                }
                _ => {}
            }
        }
        return Ok(());
    }

    // Interactive TUI mode
    tui::install_panic_hook();

    let mut terminal = tui::init().map_err(|e| anyhow::anyhow!("Failed to init terminal: {}", e))?;

    let result = run_tui(&mut terminal, engine, Some(skill_registry), Some(plugin_registry), perm_rx).await;

    // Restore terminal
    let _ = tui::restore(&mut terminal);

    result
}

async fn run_tui(
    terminal: &mut tui::TuiTerminal,
    engine: QueryEngine,
    skill_registry: Option<Arc<venus_core::skill::SkillRegistry>>,
    plugin_registry: Option<venus_core::plugin_registry::PluginRegistry>,
    perm_rx: Option<tokio::sync::mpsc::UnboundedReceiver<venus_permissions::tui_handler::PermissionRequest>>,
) -> Result<()> {
    // Create the event channel
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

    // Spawn the crossterm event poller (keyboard, mouse, resize)
    let crossterm_rx = event::spawn_event_poller();
    {
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            let mut crossterm_rx = crossterm_rx;
            while let Some(evt) = crossterm_rx.recv().await {
                if event_tx.send(evt).is_err() {
                    break;
                }
            }
        });
    }

    // Create the app with the event sender
    let mut app = app::App::new(engine.clone(), skill_registry, plugin_registry, event_tx.clone());

    // Set up cron scheduler
    let (cron_tx, mut cron_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let scheduler = std::sync::Arc::new(venus_core::cron::CronScheduler::new(
        cron_tx,
        Some(app.engine.working_dir.clone()),
    ));
    scheduler.start();
    app.engine.cron_scheduler = Some(scheduler);

    // Forward cron prompts into the event channel
    {
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            while let Some(prompt) = cron_rx.recv().await {
                if event_tx.send(event::AppEvent::CronPrompt(prompt)).is_err() {
                    break;
                }
            }
        });
    }

    // If we have a permission receiver, handle permission requests
    let mut perm_rx = perm_rx;

    // Fire SessionStart hook
    engine
        .hook_runner
        .run_simple_event(venus_core::hooks::events::HookEvent::SessionStart {
            session_id: app.engine.session_id.clone(),
            cwd: app.engine.working_dir.display().to_string(),
            model: app.engine.model.clone(),
        })
        .await;

    // Main event loop
    loop {
        // Render UI
        terminal.draw(|frame| ui::render(frame, &app))?;

        // Show/hide cursor based on input mode
        if app.input_mode == app::InputMode::Normal {
            terminal.show_cursor()?;
        } else {
            terminal.hide_cursor()?;
        }

        // Build a select over all event sources
        let evt = tokio::select! {
            // Main event channel (keyboard, tick, stream, cron)
            evt = event_rx.recv() => {
                match evt {
                    Some(e) => e,
                    None => break,
                }
            }
            // Permission requests from the engine
            Some(perm_req) = async {
                match perm_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                // Handle permission request in the TUI
                app.handle_permission_request(perm_req);
                continue;
            }
        };

        match evt {
            event::AppEvent::Key(key) => {
                app.handle_key(key).await?;
            }
            event::AppEvent::Mouse(mouse) => {
                app.handle_mouse(mouse);
            }
            event::AppEvent::Resize(_, _) => {
                // ratatui handles resize automatically
            }
            event::AppEvent::Tick => {
                app.tick();
            }
            event::AppEvent::Stream(stream_event) => {
                app.handle_stream_event(stream_event);
            }
            event::AppEvent::CronPrompt(prompt) => {
                app.handle_cron_prompt(&prompt).await?;
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Fire Stop hook
    app.engine
        .hook_runner
        .run_simple_event(venus_core::hooks::events::HookEvent::Stop {
            session_id: app.engine.session_id.clone(),
        })
        .await;

    Ok(())
}

async fn try_resume_by_prefix(
    prefix: &str,
) -> Result<Option<(venus_utils::session::SessionMeta, Vec<serde_json::Value>)>> {
    let sessions = venus_utils::session::list_sessions().await?;
    if let Some(s) = sessions.iter().find(|s| s.id.starts_with(prefix)) {
        let (meta, msgs) = venus_utils::session::load_session(&s.id).await?;
        Ok(Some((meta, msgs)))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_cli_flag_parsing() {
        let cli = Cli::try_parse_from([
            "venus",
            "--model", "claude-opus-4-20250514",
            "--max-turns", "10",
            "--max-budget-usd", "5.0",
            "--thinking", "enabled",
            "--verbose",
        ])
        .unwrap();

        assert_eq!(cli.model.as_deref(), Some("claude-opus-4-20250514"));
        assert_eq!(cli.max_turns, Some(10));
        assert_eq!(cli.max_budget_usd, Some(5.0));
        assert_eq!(cli.thinking.as_deref(), Some("enabled"));
        assert!(cli.verbose);
    }

    #[test]
    fn test_cli_default_values() {
        let cli = Cli::try_parse_from(["venus"]).unwrap();

        // model may be set via ANTHROPIC_MODEL env var, so don't assert is_none
        assert!(cli.max_turns.is_none());
        assert!(cli.max_budget_usd.is_none());
        assert!(!cli.verbose);
        assert!(!cli.dangerously_skip_permissions);
        assert!(!cli.r#continue);
        assert!(matches!(cli.output_format, OutputFormat::Text));
    }

    #[test]
    fn test_cli_permission_flags() {
        let cli = Cli::try_parse_from([
            "venus",
            "--dangerously-skip-permissions",
            "--permission-mode", "bypass",
        ])
        .unwrap();

        assert!(cli.dangerously_skip_permissions);
        assert_eq!(cli.permission_mode.as_deref(), Some("bypass"));
    }

    #[test]
    fn test_cli_tool_filtering() {
        let cli = Cli::try_parse_from([
            "venus",
            "--allowed-tools", "Bash,Read,Write",
            "--disallowed-tools", "Edit",
        ])
        .unwrap();

        assert_eq!(
            cli.allowed_tools,
            Some(vec!["Bash".to_string(), "Read".to_string(), "Write".to_string()])
        );
        assert_eq!(
            cli.disallowed_tools,
            Some(vec!["Edit".to_string()])
        );
    }

    #[test]
    fn test_cli_continue_flag() {
        let cli = Cli::try_parse_from(["venus", "--continue"]).unwrap();
        assert!(cli.r#continue);
    }

    #[test]
    fn test_cli_session_name() {
        let cli = Cli::try_parse_from(["venus", "--name", "my-session"]).unwrap();
        assert_eq!(cli.name.as_deref(), Some("my-session"));
    }

    #[test]
    fn test_cli_system_prompt_flags() {
        let cli = Cli::try_parse_from([
            "venus",
            "--append-system-prompt", "Be extra helpful.",
            "--system-prompt-file", "/tmp/prompt.txt",
        ])
        .unwrap();

        assert_eq!(
            cli.append_system_prompt.as_deref(),
            Some("Be extra helpful.")
        );
        assert_eq!(
            cli.system_prompt_file.as_deref(),
            Some(PathBuf::from("/tmp/prompt.txt").as_path())
        );
    }

    #[test]
    fn test_cli_output_format() {
        let cli = Cli::try_parse_from(["venus", "--output-format", "stream-json"]).unwrap();
        assert!(matches!(cli.output_format, OutputFormat::StreamJson));

        let cli = Cli::try_parse_from(["venus", "--output-format", "json"]).unwrap();
        assert!(matches!(cli.output_format, OutputFormat::Json));

        let cli = Cli::try_parse_from(["venus"]).unwrap();
        assert!(matches!(cli.output_format, OutputFormat::Text));
    }

    #[test]
    fn test_cli_effort_flag() {
        let cli = Cli::try_parse_from(["venus", "--effort", "high"]).unwrap();
        assert_eq!(cli.effort.as_deref(), Some("high"));
    }

    #[test]
    fn test_cli_mcp_config() {
        let cli = Cli::try_parse_from([
            "venus",
            "--mcp-config", "/path/to/mcp.json",
        ])
        .unwrap();

        assert_eq!(
            cli.mcp_config.as_deref(),
            Some(PathBuf::from("/path/to/mcp.json").as_path())
        );
    }
}
