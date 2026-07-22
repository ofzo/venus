use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, info};

use crate::background::BackgroundTaskRuntime;
use crate::engine::QueryEngine;
use crate::hooks::events::HookEvent;
use crate::hooks::HookRunner;
use crate::message::ContentBlock;
use crate::task::TaskStore;
use crate::tool::PermissionHandler;
use crate::tool_registry::ToolRegistry;
use venus_utils::config::Settings;

/// Configuration for spawning a sub-agent.
pub struct SubAgentConfig {
    pub prompt: String,
    pub description: String,
    pub model: Option<String>,
    pub working_dir: PathBuf,
    pub auth_header: &'static str,
    pub auth_value: String,
    pub base_url: String,
    pub settings: Arc<Settings>,
    pub tools: Arc<ToolRegistry>,
    pub permissions: Arc<dyn PermissionHandler>,
    pub task_store: Arc<TaskStore>,
    pub background_runtime: Arc<BackgroundTaskRuntime>,
    pub hook_runner: Arc<HookRunner>,
    /// Optional shared parent cost tracker. When `Some`, sub-agent
    /// token usage is aggregated into the parent engine's tracker.
    pub cost_tracker: Option<Arc<std::sync::Mutex<venus_utils::cost::CostTracker>>>,
}

/// Result returned by a sub-agent execution.
pub struct SubAgentResult {
    pub output: String,
    pub is_error: bool,
}

const SUBAGENT_SYSTEM_PROMPT: &str = "\
You are a sub-agent spawned to handle a specific task. \
Complete the following task and return the result. Be concise and focused. \
Do not ask for clarification — use your best judgment. \
When the task is complete, provide your final answer as plain text.";

pub struct SubAgent;

impl SubAgent {
    /// Run a sub-agent to completion and return its text output.
    pub async fn run(config: SubAgentConfig) -> Result<SubAgentResult> {
        info!(
            description = %config.description,
            "spawning sub-agent"
        );

        // Fire SubagentStart hook
        let subagent_session = uuid::Uuid::new_v4().to_string();
        let hook_runner = config.hook_runner.clone();
        hook_runner
            .run_simple_event(HookEvent::SubagentStart {
                session_id: subagent_session.clone(),
                description: config.description.clone(),
            })
            .await;

        let model = config
            .model
            .unwrap_or_else(|| config.settings.effective_model().to_string());
        let max_tokens = config.settings.effective_max_tokens();

        let engine = QueryEngine::new_for_subagent(
            config.auth_header,
            config.auth_value,
            model,
            config.base_url,
            max_tokens,
            SUBAGENT_SYSTEM_PROMPT.to_string(),
            config.tools,
            config.settings,
            config.permissions,
            config.working_dir,
            config.task_store,
            config.background_runtime,
            config.hook_runner,
            config.cost_tracker.clone(),
        );

        // Submit the prompt as a user message and run the query loop.
        // submit_message spawns the loop in the background and returns a receiver.
        let rx = engine
            .submit_message(vec![ContentBlock::text(&config.prompt)])
            .await?;

        // Drain all events until the spawned task completes.
        // This ensures the query loop has finished before we read messages.
        let mut rx = rx;
        while rx.recv().await.is_some() {}

        // Extract the final assistant response text from message history.
        let messages = engine.messages.lock().await;
        let output = extract_final_response(&messages);

        debug!(
            output_len = output.len(),
            iterations = messages.len(),
            "sub-agent completed"
        );

        // Fire SubagentStop hook
        hook_runner
            .run_simple_event(HookEvent::SubagentStop {
                session_id: subagent_session,
            })
            .await;

        Ok(SubAgentResult {
            output,
            is_error: false,
        })
    }
}

/// Extract concatenated text from the last assistant message in the conversation.
fn extract_final_response(messages: &[crate::message::Message]) -> String {
    for msg in messages.iter().rev() {
        if let crate::message::Message::Assistant(assistant) = msg {
            let texts: Vec<&str> = assistant
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            if !texts.is_empty() {
                return texts.join("\n");
            }
        }
    }
    String::from("(sub-agent produced no text output)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{AssistantMessage, Message, UserMessage};

    fn make_assistant_msg(texts: &[&str]) -> Message {
        let content: Vec<ContentBlock> = texts.iter().map(|t| ContentBlock::text(*t)).collect();
        Message::Assistant(AssistantMessage::new(content))
    }

    fn make_user_msg(text: &str) -> Message {
        Message::User(UserMessage::new(vec![ContentBlock::text(text)]))
    }

    #[test]
    fn test_extract_from_last_assistant() {
        let messages = vec![make_user_msg("question"), make_assistant_msg(&["answer"])];
        assert_eq!(extract_final_response(&messages), "answer");
    }

    #[test]
    fn test_extract_multiple_text_blocks() {
        let messages = vec![make_assistant_msg(&["part1", "part2"])];
        assert_eq!(extract_final_response(&messages), "part1\npart2");
    }

    #[test]
    fn test_extract_skips_user_messages() {
        let messages = vec![
            make_user_msg("first"),
            make_assistant_msg(&["ignored"]),
            make_user_msg("second"),
        ];
        // No assistant message at the end, so should return fallback
        // Wait - it walks backwards, so it finds the assistant message
        assert_eq!(extract_final_response(&messages), "ignored");
    }

    #[test]
    fn test_extract_empty_messages() {
        let messages = vec![];
        assert_eq!(
            extract_final_response(&messages),
            "(sub-agent produced no text output)"
        );
    }

    #[test]
    fn test_extract_no_assistant_messages() {
        let messages = vec![make_user_msg("hello"), make_user_msg("world")];
        assert_eq!(
            extract_final_response(&messages),
            "(sub-agent produced no text output)"
        );
    }

    #[test]
    fn test_extract_last_assistant_ignores_earlier() {
        let messages = vec![
            make_assistant_msg(&["first answer"]),
            make_user_msg("follow up"),
            make_assistant_msg(&["second answer"]),
        ];
        assert_eq!(extract_final_response(&messages), "second answer");
    }
}
