use std::sync::Arc;

use anyhow::Result;
use venus_core::engine::QueryEngine;
use venus_core::hooks::events::HookEvent;
use venus_core::message::ContentBlock;
use venus_core::skill::SkillRegistry;
use venus_utils::session::{self, SessionMeta};

use crate::commands::{self, CommandResult};
use crate::input::{self, InputEditor};
use crate::markdown::MarkdownRenderer;
use crate::render;

pub async fn run_repl(engine: &mut QueryEngine, skill_registry: Option<Arc<SkillRegistry>>) -> Result<()> {
    // Fire SessionStart hook
    engine
        .hook_runner
        .run_simple_event(HookEvent::SessionStart {
            session_id: engine.session_id.clone(),
            cwd: engine.working_dir.display().to_string(),
            model: engine.model.clone(),
        })
        .await;

    let history_path = input::default_history_path();
    let mut editor = InputEditor::new(history_path);

    loop {
        let line = tokio::task::block_in_place(|| editor.read_line());

        let input = match line {
            Some(s) if s.is_empty() => continue,
            Some(s) => s,
            None => {
                eprintln!("\nGoodbye!");
                break;
            }
        };

        // Handle slash commands
        if input.starts_with('/') {
            match commands::handle_command(&input, engine, skill_registry.as_ref()).await {
                CommandResult::Exit => break,
                CommandResult::InjectMessage(msg) => {
                    match submit_and_render(engine, &msg).await {
                        Ok(_) => save_current_session(engine).await,
                        Err(e) => eprintln!("\x1b[31mError: {}\x1b[0m", e),
                    }
                }
                CommandResult::Continue => {}
            }
            continue;
        }

        // Submit to engine
        match submit_and_render(engine, &input).await {
            Ok(_) => {
                save_current_session(engine).await;
            }
            Err(e) => {
                eprintln!("\x1b[31mError: {}\x1b[0m", e);
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

pub async fn run_single_prompt(engine: &mut QueryEngine, prompt: &str) -> Result<()> {
    submit_and_render(engine, prompt).await
}

async fn submit_and_render(engine: &mut QueryEngine, input: &str) -> Result<()> {
    // Run UserPromptSubmit hooks
    let effective_input;
    if let Ok(resp) = engine.hook_runner.run_user_prompt_submit(input).await {
        if resp.deny == Some(true) {
            let reason = resp.reason.unwrap_or_default();
            eprintln!(
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

    // submit_message runs the full query-tool loop and buffers events in the channel
    let mut rx = engine.submit_message(content).await?;

    // Create a markdown renderer for this response
    let mut md = MarkdownRenderer::new();

    // Drain all buffered events
    while let Some(event) = rx.recv().await {
        render::render_event(&event, &mut md);
    }

    Ok(())
}

async fn save_current_session(engine: &QueryEngine) {
    let now = chrono::Utc::now().timestamp() as u64;
    let meta = SessionMeta {
        id: engine.session_id.clone(),
        project: engine.working_dir.display().to_string(),
        created_at: engine.created_at,
        updated_at: now,
        message_count: engine.messages.len(),
        model: engine.model.clone(),
    };
    let msg_values: Vec<serde_json::Value> = engine
        .messages
        .iter()
        .filter_map(|m| serde_json::to_value(m).ok())
        .collect();
    if let Err(e) = session::save_session(&engine.session_id, &meta, &msg_values).await {
        tracing::warn!("failed to save session: {}", e);
    }
}
