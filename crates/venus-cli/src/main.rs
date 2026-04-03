mod commands;
mod render;
mod repl;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;

use venus_core::engine::QueryEngine;
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

    // Load settings
    let mut settings = Settings::load().context("failed to load settings")?;

    // CLI overrides
    if let Some(model) = cli.model {
        settings.model = Some(model);
    }
    if let Some(key) = cli.api_key {
        settings.api_key = Some(key);
    }

    let working_dir = cli
        .working_dir
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let settings = Arc::new(settings);
    let permissions = Arc::new(InteractivePermissionHandler::new());
    let tools = Arc::new(ToolRegistry::new(venus_tools::all_tools()));

    let mut engine =
        QueryEngine::new(settings.clone(), tools, permissions, working_dir.clone()).await?;

    // Print banner
    render::print_banner(&engine);

    // Non-interactive mode
    if let Some(prompt) = cli.prompt {
        repl::run_single_prompt(&mut engine, &prompt).await?;
        return Ok(());
    }

    // Interactive REPL
    repl::run_repl(&mut engine).await
}
