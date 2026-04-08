mod commands;
mod input;
mod markdown;
mod render;
mod repl;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;

use venus_core::engine::QueryEngine;
use venus_core::hooks::HookRunner;
use venus_core::skill::SkillRegistry;
use venus_core::task::TaskStore;
use venus_core::tool_registry::ToolRegistry;
use venus_permissions::interactive::InteractivePermissionHandler;
use venus_utils::config::Settings;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    let working_dir = cli
        .working_dir
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Load settings with multi-level merging (global → project → env vars)
    let mut settings =
        Settings::load_with_project(Some(&working_dir)).context("failed to load settings")?;

    // CLI overrides (highest priority)
    if let Some(model) = cli.model {
        settings.model = Some(model);
    }
    if let Some(key) = cli.api_key {
        settings.api_key = Some(key);
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
    let tools = Arc::new(ToolRegistry::new(all_tool_list));
    let task_store = Arc::new(TaskStore::new());
    let hook_runner = Arc::new(HookRunner::new(
        settings.hooks.clone(),
        String::new(),
        working_dir.clone(),
    ));

    let mut engine =
        QueryEngine::new(settings.clone(), tools, permissions, working_dir.clone(), task_store, hook_runner).await?;

    // Resume session if requested
    if let Some(resume_id) = cli.resume {
        match venus_utils::session::load_session(&resume_id).await {
            Ok((meta, msg_values)) => {
                let messages: Vec<venus_core::message::Message> = msg_values
                    .iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect();
                let msg_count = messages.len();
                engine.messages = messages;
                engine.session_id = meta.id.clone();
                engine.created_at = meta.created_at;
                eprintln!(
                    "Resumed session {} ({} messages)",
                    &meta.id[..8.min(meta.id.len())],
                    msg_count,
                );
            }
            Err(e) => {
                // Try matching by prefix
                match try_resume_by_prefix(&resume_id).await {
                    Ok(Some((meta, msg_values))) => {
                        let messages: Vec<venus_core::message::Message> = msg_values
                            .iter()
                            .filter_map(|v| serde_json::from_value(v.clone()).ok())
                            .collect();
                        let msg_count = messages.len();
                        engine.messages = messages;
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

    // Print banner
    render::print_banner(&engine);

    // Non-interactive mode
    if let Some(prompt) = cli.prompt {
        repl::run_single_prompt(&mut engine, &prompt).await?;
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
