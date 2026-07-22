use crate::message::{ContentBlock, Message};

const CLEARED_MARKER: &str = "[Old tool result content cleared]";

/// Tools whose results can be safely cleared during microcompaction.
const COMPACTABLE_TOOLS: &[&str] = &[
    "Read",
    "FileRead",
    "file_read",
    "Bash",
    "BashTerminal",
    "Grep",
    "Glob",
    "WebSearch",
    "WebFetch",
    "Edit",
    "Write",
];

/// Lightweight compaction that replaces old tool result content with a placeholder.
/// No API call is needed — this is a pure in-memory operation.
///
/// Processes messages in `messages[0..len-keep_recent]`, replacing tool result
/// content blocks from compactable tools with a short marker text.
///
/// Returns the number of content blocks cleared.
pub fn microcompact(messages: &mut [Message], keep_recent: usize) -> usize {
    let len = messages.len();
    if len <= keep_recent {
        return 0;
    }

    let cutoff = len - keep_recent;

    // First pass: collect tool_use id -> name mappings from assistant messages
    let mut tool_id_to_name = std::collections::HashMap::new();
    for msg in messages.iter().take(cutoff) {
        if let Message::Assistant(a) = msg {
            for block in &a.content {
                if let ContentBlock::ToolUse { id, name, .. } = block {
                    tool_id_to_name.insert(id.clone(), name.clone());
                }
            }
        }
    }

    // Second pass: clear compactable tool results
    let mut cleared = 0;

    for msg in messages.iter_mut().take(cutoff) {
        if let Message::User(u) = msg {
            for block in u.content.iter_mut() {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = block
                {
                    // Check if this tool result is from a compactable tool
                    let tool_name = tool_id_to_name.get(tool_use_id.as_str());
                    let is_compactable = tool_name
                        .map(|name| COMPACTABLE_TOOLS.iter().any(|t| *t == name))
                        .unwrap_or(false);

                    if !is_compactable {
                        continue;
                    }

                    // Skip if already cleared
                    let already_cleared = content.len() == 1
                        && matches!(&content[0], ContentBlock::Text { text } if text == CLEARED_MARKER);
                    if already_cleared {
                        continue;
                    }

                    *content = vec![ContentBlock::text(CLEARED_MARKER)];
                    cleared += 1;
                }
            }
        }
    }

    cleared
}

/// Check if microcompaction should trigger based on time gap.
///
/// Returns true if the time since the last assistant message exceeds
/// `threshold_secs` seconds, indicating the user has been away.
pub fn should_microcompact_by_time(messages: &[Message], threshold_secs: u64) -> bool {
    let now = chrono::Utc::now().timestamp() as u64;

    let last_assistant_ts = messages.iter().rev().find_map(|msg| match msg {
        Message::Assistant(a) => Some(a.timestamp),
        _ => None,
    });

    match last_assistant_ts {
        Some(ts) if now > ts => (now - ts) >= threshold_secs,
        _ => false,
    }
}
