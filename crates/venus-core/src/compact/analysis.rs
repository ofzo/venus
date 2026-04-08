use std::collections::HashMap;

use venus_utils::token::estimate_tokens;

use crate::message::{ContentBlock, Message};

/// Token-level breakdown of conversation context.
#[derive(Debug, Clone)]
pub struct ContextAnalysis {
    pub total_tokens: u64,
    pub system_prompt_tokens: u64,
    pub user_text_tokens: u64,
    pub assistant_text_tokens: u64,
    pub tool_request_tokens: HashMap<String, u64>,
    pub tool_result_tokens: HashMap<String, u64>,
    pub thinking_tokens: u64,
    pub message_count: usize,
    pub turn_count: usize,
    pub duplicate_file_reads: Vec<(String, usize)>,
}

/// Analyze the context window usage of a conversation.
pub fn analyze_context(messages: &[Message], system_prompt: &str) -> ContextAnalysis {
    let system_prompt_tokens = estimate_tokens(system_prompt);
    let mut user_text_tokens = 0u64;
    let mut assistant_text_tokens = 0u64;
    let mut tool_request_tokens: HashMap<String, u64> = HashMap::new();
    let mut tool_result_tokens: HashMap<String, u64> = HashMap::new();
    let mut thinking_tokens = 0u64;
    let mut turn_count = 0usize;

    // Track file reads for duplicate detection
    let mut file_read_counts: HashMap<String, usize> = HashMap::new();

    // Track which tool_use IDs map to which tool names
    let mut tool_id_to_name: HashMap<String, String> = HashMap::new();

    for msg in messages {
        match msg {
            Message::User(u) => {
                // Count as a turn if it's a genuine user message (not a tool result)
                let is_tool_result = u
                    .content
                    .first()
                    .map(|b| matches!(b, ContentBlock::ToolResult { .. }))
                    .unwrap_or(false);

                if !is_tool_result {
                    turn_count += 1;
                }

                for block in &u.content {
                    analyze_block(
                        block,
                        true,
                        &mut user_text_tokens,
                        &mut tool_request_tokens,
                        &mut tool_result_tokens,
                        &mut thinking_tokens,
                        &mut file_read_counts,
                        &tool_id_to_name,
                    );
                }
            }
            Message::Assistant(a) => {
                for block in &a.content {
                    // Collect tool_use id->name mappings
                    if let ContentBlock::ToolUse { id, name, .. } = block {
                        tool_id_to_name.insert(id.clone(), name.clone());
                    }

                    analyze_block(
                        block,
                        false,
                        &mut assistant_text_tokens,
                        &mut tool_request_tokens,
                        &mut tool_result_tokens,
                        &mut thinking_tokens,
                        &mut file_read_counts,
                        &tool_id_to_name,
                    );
                }
            }
            Message::System(_) => {}
        }
    }

    let duplicate_file_reads: Vec<(String, usize)> = file_read_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .collect();

    let total_tokens = system_prompt_tokens
        + user_text_tokens
        + assistant_text_tokens
        + tool_request_tokens.values().sum::<u64>()
        + tool_result_tokens.values().sum::<u64>()
        + thinking_tokens;

    ContextAnalysis {
        total_tokens,
        system_prompt_tokens,
        user_text_tokens,
        assistant_text_tokens,
        tool_request_tokens,
        tool_result_tokens,
        thinking_tokens,
        message_count: messages.len(),
        turn_count,
        duplicate_file_reads,
    }
}

#[allow(clippy::too_many_arguments)]
fn analyze_block(
    block: &ContentBlock,
    is_user: bool,
    text_tokens: &mut u64,
    tool_request_tokens: &mut HashMap<String, u64>,
    tool_result_tokens: &mut HashMap<String, u64>,
    thinking_tokens: &mut u64,
    file_read_counts: &mut HashMap<String, usize>,
    tool_id_to_name: &HashMap<String, String>,
) {
    match block {
        ContentBlock::Text { text } => {
            *text_tokens += estimate_tokens(text);
        }
        ContentBlock::ToolUse { name, input, .. } => {
            let input_str = serde_json::to_string(input).unwrap_or_default();
            let tokens = estimate_tokens(&input_str) + estimate_tokens(name);
            *tool_request_tokens.entry(name.clone()).or_insert(0) += tokens;

            // Track file reads for duplicate detection
            if name == "Read" || name == "FileRead" || name == "file_read" {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    *file_read_counts.entry(path.to_string()).or_insert(0) += 1;
                }
            }
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } => {
            let tool_name = tool_id_to_name
                .get(tool_use_id)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());

            for inner in content {
                if let ContentBlock::Text { text } = inner {
                    *tool_result_tokens.entry(tool_name.clone()).or_insert(0) +=
                        estimate_tokens(text);
                }
            }
        }
        ContentBlock::Thinking { thinking, .. } => {
            *thinking_tokens += estimate_tokens(thinking);
        }
    }

    // Suppress unused variable warning
    let _ = is_user;
}
