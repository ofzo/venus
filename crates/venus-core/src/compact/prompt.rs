use crate::message::{ContentBlock, Message};

/// Maximum characters to include per tool result in the summarization input.
const MAX_TOOL_RESULT_CHARS: usize = 2000;

/// Build the system prompt for the summarization API call.
pub fn summarization_system_prompt() -> String {

    r#"You are a conversation summarizer. Your job is to create a detailed summary of the conversation so far that preserves all important context needed for continuing the work.

CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.
Do NOT use Read, Bash, Grep, Glob, Edit, Write, or ANY other tool.
You already have all the context you need in the conversation above.
Your entire response must be plain text: an <analysis> block followed by a <summary> block.

Instructions:
1. First, write an <analysis> block where you chronologically analyze each part of the conversation:
   - The user's explicit requests and intents
   - Your approach and reasoning
   - Key decisions and technical concepts
   - Specific details: filenames, code snippets, function signatures, file edits
   - Errors encountered and how they were resolved
   - User feedback, especially corrections

2. Then, write a <summary> block with these sections:
   1. **Primary Request and Intent**: What the user is trying to accomplish
   2. **Key Technical Concepts**: Important technical details and decisions
   3. **Files and Code**: All file paths mentioned, with relevant code snippets
   4. **Errors and Fixes**: Problems encountered and their solutions
   5. **Current Approach**: The strategy being used
   6. **Important User Feedback**: Any corrections or preferences expressed
   7. **Pending Tasks**: Work that still needs to be done
   8. **Current State**: Where we left off, with precise details
   9. **Next Steps**: What should be done next

Be thorough and specific. Include file paths, function names, code snippets, and exact details.
The summary will replace the conversation history, so nothing important should be lost.

REMINDER: Do NOT call any tools. Respond with plain text only."#.to_string()
}

/// Serialize messages into a readable text format for the summarization API call.
///
/// Strips thinking blocks and truncates large tool results to stay within budget.
pub fn build_summary_user_message(messages: &[Message]) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push("Please summarize the following conversation:\n".to_string());
    parts.push("---\n".to_string());

    for msg in messages {
        match msg {
            Message::User(u) => {
                let is_tool_result = u
                    .content
                    .first()
                    .map(|b| matches!(b, ContentBlock::ToolResult { .. }))
                    .unwrap_or(false);

                if is_tool_result {
                    // Tool result message
                    for block in &u.content {
                        if let ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } = block
                        {
                            let label = if *is_error {
                                "Tool Error"
                            } else {
                                "Tool Result"
                            };
                            parts.push(format!("[{} for {}]:", label, tool_use_id));

                            for inner in content {
                                if let ContentBlock::Text { text } = inner {
                                    let truncated = truncate_text(text, MAX_TOOL_RESULT_CHARS);
                                    parts.push(truncated);
                                }
                            }
                            parts.push(String::new());
                        }
                    }
                } else {
                    // Genuine user message
                    parts.push("User:".to_string());
                    for block in &u.content {
                        if let ContentBlock::Text { text } = block {
                            parts.push(text.clone());
                        }
                    }
                    parts.push(String::new());
                }
            }
            Message::Assistant(a) => {
                parts.push("Assistant:".to_string());
                for block in &a.content {
                    match block {
                        ContentBlock::Text { text } => {
                            parts.push(text.clone());
                        }
                        ContentBlock::ToolUse { name, input, .. } => {
                            let input_str = serde_json::to_string_pretty(input)
                                .unwrap_or_else(|_| input.to_string());
                            let truncated = truncate_text(&input_str, MAX_TOOL_RESULT_CHARS);
                            parts.push(format!("[Tool Call: {}]\n{}", name, truncated));
                        }
                        // Skip thinking blocks — they're internal scratchpad
                        ContentBlock::Thinking { .. } => {}
                        _ => {}
                    }
                }
                parts.push(String::new());
            }
            Message::System(s) => {
                parts.push(format!("[System: {}]", s.content));
                parts.push(String::new());
            }
        }
    }

    parts.push("---".to_string());

    parts.join("\n")
}

/// Parse the summarization response, extracting the <summary> section.
///
/// Removes the <analysis> block (internal scratchpad) and extracts
/// the content between <summary> tags. Falls back to the full text
/// if no tags are found.
pub fn parse_summary(response: &str) -> String {
    // Remove <analysis>...</analysis>
    let without_analysis = if let (Some(start), Some(end)) = (
        response.find("<analysis>"),
        response.find("</analysis>"),
    ) {
        let end = end + "</analysis>".len();
        let mut result = String::new();
        result.push_str(&response[..start]);
        result.push_str(&response[end..]);
        result
    } else {
        response.to_string()
    };

    // Extract <summary>...</summary>
    if let (Some(start), Some(end)) = (
        without_analysis.find("<summary>"),
        without_analysis.find("</summary>"),
    ) {
        let content_start = start + "<summary>".len();
        without_analysis[content_start..end].trim().to_string()
    } else {
        // No tags found — use the cleaned text as-is
        without_analysis.trim().to_string()
    }
}

/// Format a summary for injection into the conversation as context.
pub fn format_compact_context(summary: &str) -> String {
    format!(
        "This conversation was compacted to reduce context length. \
         Summary of previous work:\n\n{}",
        summary
    )
}

/// Truncate text to a maximum character count, appending a marker if truncated.
fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        text.to_string()
    } else {
        format!("{}...[truncated]", &text[..max_chars])
    }
}
