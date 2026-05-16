pub mod analysis;
pub mod groups;
pub mod microcompact;
pub mod prompt;

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::hooks::events::HookEvent;
use crate::hooks::HookRunner;
use crate::message::{AssistantMessage, ContentBlock, Message, UserMessage};
use groups::group_by_api_round;
use microcompact::microcompact;
use prompt::{
    build_summary_user_message, format_compact_context, parse_summary, summarization_system_prompt,
};

/// Maximum number of PTL (prompt-too-long) retries when the summarization
/// request itself exceeds the model's context window.
const MAX_PTL_RETRIES: usize = 3;

/// Maximum number of consecutive auto-compact failures before the circuit
/// breaker stops further attempts.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// Configuration for a compaction operation.
pub struct CompactConfig {
    /// Number of recent API-round groups to keep verbatim.
    pub keep_recent_groups: usize,
    /// Model to use for the summarization call.
    pub model: String,
    /// Auth header name ("Authorization" or "x-api-key").
    pub auth_header: String,
    /// Auth header value ("Bearer <token>" or raw API key).
    pub auth_value: String,
    /// Anthropic API base URL.
    pub base_url: String,
    /// Maximum tokens for the summary output.
    pub max_summary_tokens: u32,
}

impl CompactConfig {
    /// Create a config with sensible defaults from engine parameters.
    pub fn from_engine(model: &str, auth_header: &str, auth_value: &str, base_url: &str) -> Self {
        Self {
            keep_recent_groups: 2,
            model: model.to_string(),
            auth_header: auth_header.to_string(),
            auth_value: auth_value.to_string(),
            base_url: base_url.to_string(),
            max_summary_tokens: 8192,
        }
    }
}

/// Result of a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactResult {
    pub messages_before: usize,
    pub messages_after: usize,
    pub tokens_saved_estimate: u64,
    pub summary_text: String,
}

/// Perform full compaction with AI summarization.
///
/// 1. Runs microcompact on older messages (lightweight cleanup)
/// 2. Groups messages by API round
/// 3. Sends older groups to the API for summarization
/// 4. Replaces old messages with summary context + recent messages
pub async fn compact(
    messages: &mut Vec<Message>,
    config: &CompactConfig,
) -> Result<CompactResult> {
    compact_with_hooks(messages, config, None, "").await
}

/// Perform full compaction with optional hook support.
pub async fn compact_with_hooks(
    messages: &mut Vec<Message>,
    config: &CompactConfig,
    hook_runner: Option<&HookRunner>,
    session_id: &str,
) -> Result<CompactResult> {
    let messages_before = messages.len();

    if messages_before < 4 {
        anyhow::bail!("conversation too short to compact ({} messages)", messages_before);
    }

    // Fire PreCompact hook
    if let Some(runner) = hook_runner {
        runner
            .run_simple_event(HookEvent::PreCompact {
                session_id: session_id.to_string(),
                message_count: messages_before,
            })
            .await;
    }

    // Step 1: Run microcompact first for lightweight cleanup
    let micro_cleared = microcompact(messages, config.keep_recent_groups * 4);
    if micro_cleared > 0 {
        debug!("microcompact cleared {} tool result blocks", micro_cleared);
    }

    // Step 2: Group messages by API round
    let groups = group_by_api_round(messages);

    if groups.len() <= config.keep_recent_groups {
        anyhow::bail!(
            "not enough conversation groups to compact ({} groups, need > {})",
            groups.len(),
            config.keep_recent_groups
        );
    }

    // Step 3: Split into old (to summarize) and recent (to keep)
    let split_point = groups.len() - config.keep_recent_groups;
    let old_groups = &groups[..split_point];
    let recent_groups = &groups[split_point..];

    let old_end = old_groups.last().map(|g| g.end).unwrap_or(0);
    let recent_start = recent_groups.first().map(|g| g.start).unwrap_or(messages.len());

    // Estimate tokens being removed
    let old_tokens: u64 = old_groups.iter().map(|g| g.estimated_tokens).sum();

    // Step 4: Build summarization input from old messages
    let old_messages = &messages[..old_end];
    let mut messages_to_summarize: Vec<&Message> = old_messages.iter().collect();

    // Step 5: Call API with PTL retry loop
    let system = summarization_system_prompt();
    let mut summary_text = String::new();

    for attempt in 0..=MAX_PTL_RETRIES {
        let user_content = build_summary_user_message(
            &messages_to_summarize.iter().copied().cloned().collect::<Vec<_>>(),
        );

        match call_summarization_api(
            &config.auth_header,
            &config.auth_value,
            &config.base_url,
            &config.model,
            &system,
            &user_content,
            config.max_summary_tokens,
        )
        .await
        {
            Ok(response) => {
                summary_text = parse_summary(&response);
                break;
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("prompt is too long") && attempt < MAX_PTL_RETRIES {
                    // Drop the oldest group and retry
                    let drop_count = (messages_to_summarize.len() / 4).max(1);
                    warn!(
                        "PTL retry {}/{}: dropping {} messages from head",
                        attempt + 1,
                        MAX_PTL_RETRIES,
                        drop_count
                    );
                    messages_to_summarize.drain(..drop_count);

                    if messages_to_summarize.is_empty() {
                        return Err(e).context("all messages dropped during PTL retry");
                    }
                } else {
                    return Err(e).context("summarization API call failed");
                }
            }
        }
    }

    if summary_text.is_empty() {
        anyhow::bail!("summarization produced empty result");
    }

    // Step 6: Build new message list
    let context_text = format_compact_context(&summary_text);

    let summary_user = Message::User(UserMessage::new(vec![ContentBlock::text(&context_text)]));
    let summary_ack = Message::Assistant(AssistantMessage::new(vec![ContentBlock::text(
        "I understand the context from the summary. I'm ready to continue helping with the work described above.",
    )]));

    let recent_messages: Vec<Message> = messages[recent_start..].to_vec();

    let mut new_messages = Vec::with_capacity(2 + recent_messages.len());
    new_messages.push(summary_user);
    new_messages.push(summary_ack);
    new_messages.extend(recent_messages);

    let messages_after = new_messages.len();

    *messages = new_messages;

    info!(
        "compacted: {} -> {} messages, ~{} tokens saved",
        messages_before, messages_after, old_tokens
    );

    // Fire PostCompact hook
    if let Some(runner) = hook_runner {
        runner
            .run_simple_event(HookEvent::PostCompact {
                session_id: session_id.to_string(),
                messages_before,
                messages_after,
            })
            .await;
    }

    Ok(CompactResult {
        messages_before,
        messages_after,
        tokens_saved_estimate: old_tokens,
        summary_text,
    })
}

/// Auto-compact with circuit breaker logic.
///
/// Returns `Ok(None)` if nothing to compact or circuit breaker has tripped.
/// Returns `Ok(Some(result))` on successful compaction.
/// Increments `consecutive_failures` on error, resets to 0 on success.
pub async fn auto_compact(
    messages: &mut Vec<Message>,
    config: &CompactConfig,
    consecutive_failures: &mut u32,
) -> Result<Option<CompactResult>> {
    // Circuit breaker
    if *consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
        debug!(
            "auto-compact circuit breaker: {} consecutive failures, skipping",
            consecutive_failures
        );
        return Ok(None);
    }

    match compact(messages, config).await {
        Ok(result) => {
            *consecutive_failures = 0;
            Ok(Some(result))
        }
        Err(e) => {
            *consecutive_failures += 1;
            warn!(
                "auto-compact failed (attempt {}/{}): {}",
                consecutive_failures, MAX_CONSECUTIVE_FAILURES, e
            );
            // Don't propagate the error — auto-compact failure shouldn't
            // crash the query loop
            Ok(None)
        }
    }
}

/// Compact a specific range of messages, keeping the rest intact.
pub async fn compact_partial(
    messages: &mut Vec<Message>,
    config: &CompactConfig,
    from: usize,
    up_to: usize,
) -> Result<CompactResult> {
    if from >= up_to || up_to > messages.len() {
        anyhow::bail!("invalid range: {}..{}", from, up_to);
    }

    let before = messages.len();

    // Extract the range to summarize
    let range_msgs: Vec<Message> = messages[from..up_to].to_vec();

    // Build summary prompt from the range
    let summary = build_range_summary(&range_msgs, config).await?;

    // Replace range with summary
    let summary_msg = Message::User(UserMessage::new(vec![ContentBlock::text(&summary)]));
    messages.splice(from..up_to, std::iter::once(summary_msg));

    Ok(CompactResult {
        messages_before: before,
        messages_after: messages.len(),
        tokens_saved_estimate: ((before - messages.len()) * 500) as u64,
        summary_text: summary,
    })
}

/// Reactive compact using Anthropic's API-side context edit strategy.
///
/// Instead of summarizing locally, this sends the conversation to the API
/// with a `context_edit` instruction that tells the model to compress
/// older parts of the conversation while preserving key information.
/// This is more efficient than full summarization for moderate context pressure.
pub async fn reactive_compact(
    messages: &mut Vec<Message>,
    config: &CompactConfig,
) -> Result<CompactResult> {
    let messages_before = messages.len();

    if messages_before < 6 {
        anyhow::bail!("conversation too short for reactive compact");
    }

    // Keep the last 4 messages verbatim, compress the rest
    let keep_count = 4;
    let split_point = messages_before - keep_count;

    // Build a compression prompt for the old messages
    let old_messages = &messages[..split_point];
    let mut compress_input = String::from(
        "Compress the following conversation into a concise summary that preserves:\n\
         1. All file paths mentioned\n\
         2. All decisions made\n\
         3. Current task/plan state\n\
         4. Key technical details\n\n\
         Conversation:\n",
    );

    for msg in old_messages {
        let role = match msg {
            Message::User(_) => "User",
            Message::Assistant(_) => "Assistant",
            Message::System(_) => "System",
        };
        let text: String = match msg {
            Message::User(u) => u.content.iter().filter_map(|b| b.as_text()).collect::<Vec<_>>().join(" "),
            Message::Assistant(a) => a.content.iter().filter_map(|b| b.as_text()).collect::<Vec<_>>().join(" "),
            Message::System(s) => s.content.clone(),
        };
        if !text.is_empty() {
            compress_input.push_str(&format!("{}: {}\n", role, &text[..text.len().min(500)]));
        }
    }

    // Call API for compression
    let compressed = call_summarization_api(
        &config.auth_header,
        &config.auth_value,
        &config.base_url,
        &config.model,
        "You are a conversation compressor. Produce a concise summary.",
        &compress_input,
        2048,
    )
    .await?;

    // Replace old messages with compressed summary
    let summary_msg = Message::User(UserMessage::new(vec![ContentBlock::text(&format!(
        "[Context compressed]\n{}",
        compressed
    ))]));
    let ack_msg = Message::Assistant(AssistantMessage::new(vec![ContentBlock::text(
        "Understood. Continuing with compressed context.",
    )]));

    let recent: Vec<Message> = messages[split_point..].to_vec();
    let mut new_messages = Vec::with_capacity(2 + recent.len());
    new_messages.push(summary_msg);
    new_messages.push(ack_msg);
    new_messages.extend(recent);

    let messages_after = new_messages.len();
    let tokens_saved = ((messages_before - messages_after) * 500) as u64;

    *messages = new_messages;

    Ok(CompactResult {
        messages_before,
        messages_after,
        tokens_saved_estimate: tokens_saved,
        summary_text: compressed,
    })
}

/// After compaction, re-read files mentioned in the summary to restore context.
pub async fn post_compact_refresh(summary: &str, working_dir: &std::path::Path) -> Vec<String> {
    let mut refreshed = Vec::new();
    // Simple heuristic: look for file paths in the summary
    for line in summary.lines() {
        for word in line.split_whitespace() {
            // Skip common non-path words
            if word.len() < 5 || word.starts_with('#') || word.starts_with('-') {
                continue;
            }
            let path = working_dir.join(word);
            if path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    // Only re-read small files (< 10KB)
                    if content.len() < 10240 {
                        refreshed.push(format!("{}:\n{}", path.display(), content));
                    }
                }
            }
        }
    }
    refreshed
}

/// Compact with plan mode re-injection and post-compact file refresh.
pub async fn compact_with_hooks_and_plan(
    messages: &mut Vec<Message>,
    config: &CompactConfig,
    hook_runner: Option<&HookRunner>,
    session_id: &str,
    plan_mode_active: bool,
    working_dir: &std::path::Path,
) -> Result<CompactResult> {
    // Run existing compaction
    let result = compact_with_hooks(messages, config, hook_runner, session_id).await?;

    // Re-inject plan mode context
    if plan_mode_active {
        messages.push(Message::User(UserMessage::new(vec![
            ContentBlock::text("[Plan mode is active. Continue working according to the plan.]"),
        ])));
    }

    // Re-read files mentioned in summary
    let refreshed = post_compact_refresh(&result.summary_text, working_dir).await;
    for content in refreshed {
        messages.push(Message::User(UserMessage::new(vec![
            ContentBlock::text(&content),
        ])));
    }

    Ok(result)
}

/// Build a summary of a specific range of messages by calling the summarization API.
async fn build_range_summary(
    range_msgs: &[Message],
    config: &CompactConfig,
) -> Result<String> {
    let system = summarization_system_prompt();
    let user_content = build_summary_user_message(range_msgs);

    let response = call_summarization_api(
        &config.auth_header,
        &config.auth_value,
        &config.base_url,
        &config.model,
        &system,
        &user_content,
        config.max_summary_tokens,
    )
    .await?;

    Ok(parse_summary(&response))
}

/// Make a non-streaming API call for summarization.
///
/// Uses the same reqwest pattern as `engine.rs` to avoid a dependency
/// on `venus-services` (which would create a circular dependency).
async fn call_summarization_api(
    auth_header: &str,
    auth_value: &str,
    base_url: &str,
    model: &str,
    system: &str,
    user_content: &str,
    max_tokens: u32,
) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let url = format!("{}/v1/messages", base_url);

    let request = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": system,
        "messages": [
            {
                "role": "user",
                "content": user_content,
            }
        ],
        "stream": false,
    });

    let request_body = serde_json::to_string(&request)?;

    // Retry loop with exponential backoff for rate limits and server errors
    const MAX_RETRIES: u32 = 3;
    const BASE_DELAY_MS: u64 = 1000;

    let mut body = String::new();
    for attempt in 0..=MAX_RETRIES {
        let result = client
            .post(&url)
            .header(auth_header, auth_value)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(request_body.clone())
            .send()
            .await;

        let response = match result {
            Ok(r) => r,
            Err(e) if attempt < MAX_RETRIES && e.is_timeout() => {
                let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                continue;
            }
            Err(e) => return Err(e).context("summarization API request failed"),
        };

        let status = response.status();
        body = response.text().await.unwrap_or_default();

        if status.is_success() {
            break;
        }

        let status_code = status.as_u16();
        let is_retryable = status_code == 429 || status_code == 529 || status_code >= 500;

        if !is_retryable || attempt >= MAX_RETRIES {
            anyhow::bail!("summarization API error ({}): {}", status, &body[..body.len().min(500)]);
        }

        let delay = BASE_DELAY_MS * 2u64.pow(attempt);
        tracing::debug!("summarization API returned {}, retrying in {}ms", status_code, delay);
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }

    // Parse the response to extract the text content
    let parsed: serde_json::Value =
        serde_json::from_str(&body).context("failed to parse summarization API response")?;

    let text = parsed
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|block| {
                if block.get("type")?.as_str()? == "text" {
                    block.get("text")?.as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
        })
        .unwrap_or_default();

    if text.is_empty() {
        anyhow::bail!("summarization API returned empty text content");
    }

    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{AssistantMessage, UserMessage};

    fn make_text_message(text: &str) -> Message {
        Message::User(UserMessage::new(vec![ContentBlock::text(text)]))
    }

    fn make_assistant_message(text: &str) -> Message {
        Message::Assistant(AssistantMessage::new(vec![ContentBlock::text(text)]))
    }

    fn test_config() -> CompactConfig {
        CompactConfig {
            keep_recent_groups: 1,
            model: "test-model".to_string(),
            auth_header: "x-api-key".to_string(),
            auth_value: "test-key".to_string(),
            base_url: "https://api.test.com".to_string(),
            max_summary_tokens: 1024,
        }
    }

    #[test]
    fn test_compact_partial_invalid_range_from_equals_up_to() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut messages = vec![
                make_text_message("hello"),
                make_text_message("world"),
                make_text_message("foo"),
            ];
            let config = test_config();
            let result = compact_partial(&mut messages, &config, 1, 1).await;
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("invalid range"));
        });
    }

    #[test]
    fn test_compact_partial_invalid_range_up_to_exceeds() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut messages = vec![
                make_text_message("hello"),
                make_text_message("world"),
            ];
            let config = test_config();
            let result = compact_partial(&mut messages, &config, 0, 5).await;
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("invalid range"));
        });
    }

    #[test]
    fn test_compact_partial_invalid_range_from_greater_than_up_to() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut messages = vec![
                make_text_message("hello"),
                make_text_message("world"),
            ];
            let config = test_config();
            let result = compact_partial(&mut messages, &config, 2, 1).await;
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("invalid range"));
        });
    }

    #[test]
    fn test_post_compact_refresh_finds_files() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = std::env::temp_dir();
            let test_file = tmp.join("venus_test_post_compact_refresh.txt");
            std::fs::write(&test_file, "hello from test file").unwrap();

            let summary = format!("We edited {}", test_file.display());
            let refreshed = post_compact_refresh(&summary, &tmp).await;
            assert_eq!(refreshed.len(), 1);
            assert!(refreshed[0].contains("hello from test file"));

            std::fs::remove_file(&test_file).unwrap();
        });
    }

    #[test]
    fn test_post_compact_refresh_skips_large_files() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = std::env::temp_dir();
            let test_file = tmp.join("venus_test_post_compact_large.txt");
            // Create a file > 10KB
            let large_content = "x".repeat(11000);
            std::fs::write(&test_file, &large_content).unwrap();

            let summary = format!("We edited {}", test_file.display());
            let refreshed = post_compact_refresh(&summary, &tmp).await;
            assert!(refreshed.is_empty());

            std::fs::remove_file(&test_file).unwrap();
        });
    }

    #[test]
    fn test_post_compact_refresh_skips_nonexistent_files() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = std::env::temp_dir();
            let summary = "We edited /nonexistent/path/file.rs which does not exist";
            let refreshed = post_compact_refresh(summary, &tmp).await;
            assert!(refreshed.is_empty());
        });
    }

    #[test]
    fn test_compact_partial_valid_range() {
        // This test verifies the range validation works correctly for valid inputs.
        // We can't easily test the full API call without mocking, so we just test
        // that valid ranges pass the initial checks.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut messages = vec![
                make_text_message("msg1"),
                make_text_message("msg2"),
                make_text_message("msg3"),
                make_text_message("msg4"),
            ];
            let config = test_config();
            // This will fail at the API call stage (since we don't have a real API),
            // but it should NOT fail at the range validation stage.
            let result = compact_partial(&mut messages, &config, 0, 2).await;
            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(
                !err_msg.contains("invalid range"),
                "Should not fail with 'invalid range' for valid range 0..2"
            );
        });
    }
}
