mod commands;
mod input;
mod markdown;
mod render;
mod repl;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;

use venus_core::background::BackgroundTaskRuntime;
use venus_core::engine::QueryEngine;
use venus_core::hooks::HookRunner;
use venus_core::skill::SkillRegistry;
use venus_core::task::TaskStore;
use venus_core::tool_registry::ToolRegistry;
use venus_permissions::interactive::InteractivePermissionHandler;
use venus_utils::config::Settings;

#[derive(clap::ValueEnum, Clone, Debug, Default)]
enum OutputFormat {
    #[default]
    Text,
    Json,
    StreamJson,
}

#[derive(Parser, Debug)]
#[command(name = "venus", about = "Venus - AI coding assistant")]
struct Cli {
    /// Model to use
    #[arg(short, long, env = "ANTHROPIC_MODEL")]
    model: Option<String>,

    /// API key
    #[arg(short = 'k', long, env = "ANTHROPIC_API_KEY")]
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
        settings.api_key = Some(key);
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

    // --permission-mode: override settings.permission_mode
    if let Some(ref mode) = cli.permission_mode {
        settings.permission_mode = Some(mode.clone());
    }

    // --dangerously-skip-permissions: set permission_mode to bypass
    if cli.dangerously_skip_permissions {
        settings.permission_mode = Some("bypass".into());
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
    let permissions = Arc::new(InteractivePermissionHandler::new(settings.clone()));

    // Load skills from ~/.claude/skills/ and <project>/.claude/skills/
    let skill_dirs = vec![
        dirs::home_dir()
            .unwrap_or_default()
            .join(".claude")
            .join("skills"),
        working_dir.join(".claude").join("skills"),
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
            .join(".claude")
            .join("plugins"),
        working_dir.join(".claude").join("plugins"),
    ];
    let mut plugin_registry = venus_core::plugin_registry::PluginRegistry::new();
    if let Err(e) = plugin_registry.load_all(&plugin_dirs).await {
        eprintln!("Warning: failed to load plugins: {}", e);
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
                    eprintln!("Warning: failed to start MCP servers: {}", e);
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    let tools = Arc::new(ToolRegistry::new(all_tool_list));
    let task_store = Arc::new(TaskStore::new());
    let background_runtime = Arc::new(BackgroundTaskRuntime::new());
    let hook_runner = Arc::new(HookRunner::new(
        settings.hooks.clone(),
        String::new(),
        working_dir.clone(),
    ));

    let mut engine =
        QueryEngine::new(settings.clone(), tools, permissions, working_dir.clone(), task_store, background_runtime, hook_runner).await?;

    // Apply CLI flags to engine
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
    // Store tool filtering flags for runtime use (logged via verbose mode)
    if cli.allowed_tools.is_some() || cli.disallowed_tools.is_some() {
        tracing::debug!(
            allowed = ?cli.allowed_tools,
            disallowed = ?cli.disallowed_tools,
            "tool filtering flags specified"
        );
    }
    if let Some(ref name) = cli.name {
        tracing::debug!(session_name = %name, "session name specified");
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
                eprintln!(
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
                        eprintln!(
                            "Resumed session {} ({} messages)",
                            &meta.id[..8.min(meta.id.len())],
                            msg_count,
                        );
                    }
                    _ => {
                        eprintln!("Warning: could not resume session '{}': {}", resume_id, e);
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
                            eprintln!(
                                "Continued session {} ({} messages)",
                                &meta.id[..8.min(meta.id.len())],
                                msg_count,
                            );
                        }
                        Err(e) => {
                            eprintln!("Warning: failed to load latest session: {}", e);
                        }
                    }
                } else {
                    eprintln!("No saved sessions to continue.");
                }
            }
            Err(e) => {
                eprintln!("Warning: failed to list sessions: {}", e);
            }
        }
    }

    // Print banner
    render::print_banner(&engine);

    // Non-interactive mode
    if let Some(prompt) = cli.prompt {
        repl::run_single_prompt(&engine, &prompt).await?;
        return Ok(());
    }

    // Interactive REPL
    repl::run_repl(&mut engine, Some(skill_registry)).await
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
