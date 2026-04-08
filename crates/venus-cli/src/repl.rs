use anyhow::Result;
use venus_core::engine::QueryEngine;
use venus_core::message::ContentBlock;
use std::io::{self, BufRead, Write};

use crate::commands;
use crate::render;

pub async fn run_repl(engine: &mut QueryEngine) -> Result<()> {
    let stdin = io::stdin();

    loop {
        // Print prompt
        eprint!("\x1b[1;32m> \x1b[0m");
        io::stderr().flush()?;

        // Read input line
        let mut input = String::new();
        let bytes = stdin.lock().read_line(&mut input)?;

        // EOF (Ctrl+D)
        if bytes == 0 {
            eprintln!("\nGoodbye!");
            break;
        }

        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }

        // Handle slash commands
        if input.starts_with('/') {
            let should_exit = commands::handle_command(&input, engine).await;
            if should_exit {
                break;
            }
            continue;
        }

        // Submit to engine
        match submit_and_render(engine, &input).await {
            Ok(_) => {}
            Err(e) => {
                eprintln!("\x1b[31mError: {}\x1b[0m", e);
            }
        }
    }

    Ok(())
}

pub async fn run_single_prompt(engine: &mut QueryEngine, prompt: &str) -> Result<()> {
    submit_and_render(engine, prompt).await
}

async fn submit_and_render(engine: &mut QueryEngine, input: &str) -> Result<()> {
    let content = vec![ContentBlock::text(input)];

    // submit_message runs the full query-tool loop and buffers events in the channel
    let mut rx = engine.submit_message(content).await?;

    // Drain all buffered events
    while let Some(event) = rx.recv().await {
        render::render_event(&event);
    }

    Ok(())
}
