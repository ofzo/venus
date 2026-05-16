use std::io::{self, Write};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;
use venus_core::engine::QueryEngine;
use venus_core::hooks::events::HookEvent;
use venus_core::message::ContentBlock;
use venus_core::skill::SkillRegistry;
use venus_utils::session::{self, SessionMeta};

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

use crate::commands::{self, CommandResult};
use crate::input;
use crate::markdown::MarkdownRenderer;
use crate::render;

pub async fn run_repl(engine: &mut QueryEngine, skill_registry: Option<Arc<SkillRegistry>>) -> Result<()> {
    // Create cron channel and scheduler
    let (cron_tx, mut cron_rx) = mpsc::unbounded_channel::<String>();

    let scheduler = Arc::new(venus_core::cron::CronScheduler::new(
        cron_tx,
        Some(engine.working_dir.clone()),
    ));
    scheduler.start();
    engine.cron_scheduler = Some(scheduler);

    // Fire SessionStart hook
    engine
        .hook_runner
        .run_simple_event(HookEvent::SessionStart {
            session_id: engine.session_id.clone(),
            cwd: engine.working_dir.display().to_string(),
            model: engine.model.clone(),
        })
        .await;

    // Move input reading to a dedicated thread
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Option<String>>();
    let (vim_tx, mut vim_rx) = mpsc::unbounded_channel::<bool>();

    let history_path = input::default_history_path();
    std::thread::spawn(move || {
        let mut editor = input::InputEditor::new(history_path);
        loop {
            // Check for vim toggle signals (non-blocking)
            if let Ok(toggle) = vim_rx.try_recv() {
                if toggle {
                    editor.toggle_vim_mode();
                }
            }
            match editor.read_line() {
                Some(line) => {
                    if input_tx.send(Some(line)).is_err() {
                        break;
                    }
                }
                None => {
                    // EOF (Ctrl+D) or error
                    input_tx.send(None).ok();
                    break;
                }
            }
        }
    });

    // Main event loop
    loop {
        tokio::select! {
            maybe_input = input_rx.recv() => {
                match maybe_input {
                    Some(Some(line)) => {
                        if line.is_empty() {
                            continue;
                        }

                        if line.starts_with('/') {
                            match commands::handle_command(&line, engine, skill_registry.as_ref()).await {
                                CommandResult::Exit => break,
                                CommandResult::InjectMessage(msg) => {
                                    match submit_and_render(engine, &msg).await {
                                        Ok(_) => save_current_session(engine).await,
                                        Err(e) => eprintlf!("\x1b[31mError: {}\x1b[0m", e),
                                    }
                                }
                                CommandResult::ToggleVim => {
                                    vim_tx.send(true).ok();
                                    eprintlf!("  Vim mode toggled.");
                                }
                                CommandResult::Continue => {}
                            }
                            continue;
                        }

                        match submit_and_render(engine, &line).await {
                            Ok(_) => {
                                save_current_session(engine).await;
                            }
                            Err(e) => {
                                eprintlf!("\x1b[31mError: {}\x1b[0m", e);
                            }
                        }
                    }
                    Some(None) | None => {
                        // EOF or channel closed
                        eprintlf!("\r\nGoodbye!");
                        break;
                    }
                }
            }
            Some(cron_prompt) = cron_rx.recv() => {
                eprintlf!("\r\n  [cron] Executing scheduled prompt...");
                if let Err(e) = submit_and_render(engine, &cron_prompt).await {
                    eprintlf!("\x1b[31mCron error: {}\x1b[0m", e);
                }
                save_current_session(engine).await;
            }
        }
    }

    // Fire Stop hook
    engine
        .hook_runner
        .run_simple_event(HookEvent::Stop {
            session_id: engine.session_id.clone(),
        })
        .await;

    Ok(())
}

pub async fn run_single_prompt(engine: &QueryEngine, prompt: &str) -> Result<()> {
    submit_and_render(engine, prompt).await
}

async fn submit_and_render(engine: &QueryEngine, input: &str) -> Result<()> {
    // Run UserPromptSubmit hooks
    let effective_input;
    if let Ok(resp) = engine.hook_runner.run_user_prompt_submit(input).await {
        if resp.deny == Some(true) {
            let reason = resp.reason.unwrap_or_default();
            eprintlf!(
                "  \x1b[33mBlocked by hook: {}\x1b[0m",
                if reason.is_empty() {
                    "denied"
                } else {
                    &reason
                }
            );
            return Ok(());
        }
        effective_input = resp
            .updated_prompt
            .unwrap_or_else(|| input.to_string());
    } else {
        effective_input = input.to_string();
    }

    let content = vec![ContentBlock::text(&effective_input)];

    // submit_message spawns the query loop and returns a streaming receiver
    let mut rx = engine.submit_message(content).await?;

    // Create a markdown renderer for this response
    let mut md = MarkdownRenderer::new();

    // Drain all events from the streaming receiver
    while let Some(event) = rx.recv().await {
        render::render_event(&event, &mut md);
    }

    Ok(())
}

async fn save_current_session(engine: &QueryEngine) {
    let now = chrono::Utc::now().timestamp() as u64;
    let messages = engine.messages.lock().await;
    let meta = SessionMeta {
        id: engine.session_id.clone(),
        project: engine.working_dir.display().to_string(),
        created_at: engine.created_at,
        updated_at: now,
        message_count: messages.len(),
        model: engine.model.clone(),
    };
    let msg_values: Vec<serde_json::Value> = messages
        .iter()
        .filter_map(|m| serde_json::to_value(m).ok())
        .collect();
    drop(messages);
    if let Err(e) = session::save_session(&engine.session_id, &meta, &msg_values).await {
        tracing::warn!("failed to save session: {}", e);
    }
}
