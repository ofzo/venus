use venus_utils::token::estimate_tokens;

use crate::message::{ContentBlock, Message};

/// A group of messages representing one API round-trip.
#[derive(Debug, Clone)]
pub struct MessageGroup {
    /// Start index in the messages array (inclusive).
    pub start: usize,
    /// End index in the messages array (exclusive).
    pub end: usize,
    /// Estimated token count for all messages in this group.
    pub estimated_tokens: u64,
}

/// Group messages into API rounds.
///
/// Each group starts with a genuine user message (not a tool result continuation)
/// and includes the assistant response plus any subsequent tool-result user messages
/// until the next genuine user message.
pub fn group_by_api_round(messages: &[Message]) -> Vec<MessageGroup> {
    if messages.is_empty() {
        return Vec::new();
    }

    let mut groups: Vec<MessageGroup> = Vec::new();
    let mut current_start: Option<usize> = None;

    for (i, msg) in messages.iter().enumerate() {
        let is_genuine_user_turn = match msg {
            Message::User(u) => {
                // A genuine user turn is a user message whose first content block
                // is NOT a ToolResult (tool results are continuations of tool calls)
                !u.content
                    .first()
                    .map(|b| matches!(b, ContentBlock::ToolResult { .. }))
                    .unwrap_or(false)
            }
            _ => false,
        };

        if is_genuine_user_turn {
            // Close the previous group
            if let Some(start) = current_start {
                let tokens = estimate_group_tokens(&messages[start..i]);
                groups.push(MessageGroup {
                    start,
                    end: i,
                    estimated_tokens: tokens,
                });
            }
            current_start = Some(i);
        } else if current_start.is_none() {
            // Handle messages before the first genuine user turn (e.g., System messages)
            current_start = Some(i);
        }
    }

    // Close the last group
    if let Some(start) = current_start {
        let tokens = estimate_group_tokens(&messages[start..]);
        groups.push(MessageGroup {
            start,
            end: messages.len(),
            estimated_tokens: tokens,
        });
    }

    groups
}

/// Estimate total tokens for a slice of messages.
fn estimate_group_tokens(messages: &[Message]) -> u64 {
    let mut total = 0u64;
    for msg in messages {
        match msg {
            Message::User(u) => {
                total += estimate_content_tokens(&u.content);
            }
            Message::Assistant(a) => {
                total += estimate_content_tokens(&a.content);
            }
            Message::System(s) => {
                total += estimate_tokens(&s.content);
            }
        }
    }
    total
}

/// Estimate tokens for a list of content blocks.
fn estimate_content_tokens(blocks: &[ContentBlock]) -> u64 {
    let mut total = 0u64;
    for block in blocks {
        match block {
            ContentBlock::Text { text } => {
                total += estimate_tokens(text);
            }
            ContentBlock::ToolUse { name, input, .. } => {
                total += estimate_tokens(name);
                let input_str = serde_json::to_string(input).unwrap_or_default();
                total += estimate_tokens(&input_str);
            }
            ContentBlock::ToolResult { content, .. } => {
                total += estimate_content_tokens(content);
            }
            ContentBlock::Thinking { thinking } => {
                total += estimate_tokens(thinking);
            }
        }
    }
    total
}
