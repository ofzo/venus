use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use venus_utils::claudemd;
use venus_utils::config::Settings;
use venus_utils::cost::CostTracker;
use venus_utils::git;

use crate::message::*;
use crate::stream::StreamEvent;
use crate::task::TaskStore;
use crate::tool::{PermissionDecision, PermissionHandler, ToolContext, ToolResult};
use crate::tool_registry::ToolRegistry;

pub struct QueryEngine {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub max_tokens: u32,
    pub messages: Vec<Message>,
    pub tools: Arc<ToolRegistry>,
    pub settings: Arc<Settings>,
    pub permissions: Arc<dyn PermissionHandler>,
    pub cost_tracker: CostTracker,
    pub cancel_token: CancellationToken,
    pub working_dir: PathBuf,
    pub system_prompt: String,
    pub task_store: Arc<TaskStore>,
}

impl QueryEngine {
    pub async fn new(
        settings: Arc<Settings>,
        tools: Arc<ToolRegistry>,
        permissions: Arc<dyn PermissionHandler>,
        working_dir: PathBuf,
        task_store: Arc<TaskStore>,
    ) -> Result<Self> {
        let api_key = settings
            .api_key
            .clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .context("API key required: set ANTHROPIC_API_KEY or configure api_key in settings")?;

        let model = settings.effective_model().to_string();
        let base_url = settings.effective_base_url().to_string();
        let max_tokens = settings.effective_max_tokens();

        // Build system prompt
        let system_prompt = build_system_prompt(&working_dir).await;

        Ok(Self {
            api_key,
            model,
            base_url,
            max_tokens,
            messages: Vec::new(),
            tools,
            settings,
            permissions,
            cost_tracker: CostTracker::new(),
            cancel_token: CancellationToken::new(),
            working_dir,
            system_prompt,
            task_store,
        })
    }

    /// Submit a user message and process the full query-tool loop.
    /// Events are sent through the returned receiver.
    pub async fn submit_message(
        &mut self,
        content: Vec<ContentBlock>,
    ) -> Result<mpsc::UnboundedReceiver<StreamEvent>> {
        let user_msg = UserMessage::new(content);
        self.messages.push(Message::User(user_msg));

        let (tx, rx) = mpsc::unbounded_channel();

        // Run the query loop
        self.run_query_loop(tx).await?;

        Ok(rx)
    }

    async fn run_query_loop(&mut self, tx: mpsc::UnboundedSender<StreamEvent>) -> Result<()> {
        let max_iterations = 25; // prevent infinite loops

        for iteration in 0..max_iterations {
            debug!("query loop iteration {}", iteration);

            // Build API request
            let api_messages = messages_to_api_params(&self.messages);
            let tool_defs = self.tools.api_definitions();

            let request = serde_json::json!({
                "model": self.model,
                "max_tokens": self.max_tokens,
                "system": self.system_prompt,
                "messages": api_messages,
                "tools": tool_defs,
                "stream": true,
            });

            // Make streaming API call
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()?;

            let url = format!("{}/v1/messages", self.base_url);
            let response = client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .body(serde_json::to_string(&request)?)
                .send()
                .await
                .context("API request failed")?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                let err_msg = format!("API error ({}): {}", status, &body[..body.len().min(500)]);
                tx.send(StreamEvent::Error(err_msg.clone())).ok();
                return Err(anyhow::anyhow!(err_msg));
            }

            // Parse SSE stream
            let assistant_msg = self.process_sse_stream(response, &tx).await?;

            let stop_reason = assistant_msg.stop_reason.clone();

            // Record usage
            if let Some(usage) = &assistant_msg.usage {
                self.cost_tracker.record(&self.model, usage);
                tx.send(StreamEvent::Usage(usage.clone())).ok();
            }

            // Check for tool use
            let tool_calls: Vec<(String, String, serde_json::Value)> = assistant_msg
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, name, input } => {
                        Some((id.clone(), name.clone(), input.clone()))
                    }
                    _ => None,
                })
                .collect();

            // Add assistant message to history
            self.messages.push(Message::Assistant(assistant_msg));

            if tool_calls.is_empty()
                || stop_reason.as_deref() == Some("end_turn") && tool_calls.is_empty()
            {
                // No tool calls — we're done
                break;
            }

            // Execute tool calls
            for (id, name, input) in &tool_calls {
                let result = self.execute_tool(id, name, input, &tx).await;

                // Add tool result to messages
                let tool_result_msg =
                    Message::User(UserMessage::new(vec![ContentBlock::tool_result(
                        id.clone(),
                        result.content.clone(),
                        result.is_error,
                    )]));
                self.messages.push(tool_result_msg);
            }

            // Continue the loop for another API call
        }

        Ok(())
    }

    async fn execute_tool(
        &self,
        id: &str,
        name: &str,
        input: &serde_json::Value,
        tx: &mpsc::UnboundedSender<StreamEvent>,
    ) -> ToolResult {
        // Check permission
        let decision = self.permissions.check_permission(name, input).await;
        match decision {
            PermissionDecision::Allow => {}
            PermissionDecision::Deny(reason) => {
                let result = ToolResult::error(format!("Permission denied: {}", reason));
                tx.send(StreamEvent::ToolResult {
                    id: id.to_string(),
                    name: name.to_string(),
                    result: result.clone(),
                })
                .ok();
                return result;
            }
            PermissionDecision::Ask(_description) => {
                // For interactive mode, the permission handler already prompted
                // If we get here, it means it was allowed
            }
        }

        // Find and execute the tool
        let tool = match self.tools.find_by_name(name) {
            Some(t) => t,
            None => {
                let result = ToolResult::error(format!("unknown tool: {}", name));
                tx.send(StreamEvent::ToolResult {
                    id: id.to_string(),
                    name: name.to_string(),
                    result: result.clone(),
                })
                .ok();
                return result;
            }
        };

        let ctx = ToolContext {
            working_dir: self.working_dir.clone(),
            session_id: "session".to_string(),
            cancel_token: self.cancel_token.clone(),
            permission_handler: self.permissions.clone(),
            settings: self.settings.clone(),
            task_store: self.task_store.clone(),
        };

        info!("executing tool: {} with input: {}", name, input);

        let result = match tool.execute(input.clone(), &ctx).await {
            Ok(r) => r,
            Err(e) => ToolResult::error(format!("tool error: {}", e)),
        };

        tx.send(StreamEvent::ToolResult {
            id: id.to_string(),
            name: name.to_string(),
            result: result.clone(),
        })
        .ok();

        result
    }

    async fn process_sse_stream(
        &self,
        response: reqwest::Response,
        tx: &mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<AssistantMessage> {
        use venus_utils::cost::TokenUsage;
        use futures_util::StreamExt;

        let mut parser = SseParserInline::new();
        let byte_stream = response.bytes_stream();
        let mut pinned = std::pin::pin!(byte_stream);

        let mut model = String::new();
        let mut blocks: Vec<BlockBuilder> = Vec::new();
        let mut stop_reason: Option<String> = None;
        let mut total_usage = TokenUsage::default();

        while let Some(chunk) = pinned.next().await {
            let chunk = chunk?;
            parser.buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(raw) = parser.next_raw_event() {
                let (event_type, data) = raw;
                if data.is_empty() {
                    continue;
                }

                let parsed: serde_json::Value = match serde_json::from_str(&data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                match event_type.as_str() {
                    "message_start" => {
                        if let Some(msg) = parsed.get("message") {
                            model = msg
                                .get("model")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if let Some(u) = msg.get("usage") {
                                total_usage.input_tokens =
                                    u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                                total_usage.cache_read_tokens = u
                                    .get("cache_read_input_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                total_usage.cache_creation_tokens = u
                                    .get("cache_creation_input_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                            }
                        }
                    }
                    "content_block_start" => {
                        let index =
                            parsed.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        while blocks.len() <= index {
                            blocks.push(BlockBuilder::default());
                        }

                        if let Some(cb) = parsed.get("content_block") {
                            let block_type =
                                cb.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                            match block_type {
                                "text" => blocks[index].kind = BKind::Text,
                                "tool_use" => {
                                    blocks[index].kind = BKind::ToolUse;
                                    blocks[index].tool_id = cb
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    blocks[index].tool_name = cb
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    tx.send(StreamEvent::ToolUseStart {
                                        id: blocks[index].tool_id.clone().unwrap_or_default(),
                                        name: blocks[index].tool_name.clone().unwrap_or_default(),
                                    })
                                    .ok();
                                }
                                "thinking" => blocks[index].kind = BKind::Thinking,
                                _ => {}
                            }
                        }
                    }
                    "content_block_delta" => {
                        let index =
                            parsed.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        if index < blocks.len() {
                            if let Some(delta) = parsed.get("delta") {
                                let delta_type =
                                    delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                match delta_type {
                                    "text_delta" => {
                                        let text = delta
                                            .get("text")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        blocks[index].text.push_str(text);
                                        tx.send(StreamEvent::TextDelta(text.to_string())).ok();
                                    }
                                    "input_json_delta" => {
                                        let json = delta
                                            .get("partial_json")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        blocks[index].text.push_str(json);
                                        tx.send(StreamEvent::ToolUseInput(json.to_string())).ok();
                                    }
                                    "thinking_delta" => {
                                        let thinking = delta
                                            .get("thinking")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        blocks[index].text.push_str(thinking);
                                        tx.send(StreamEvent::ThinkingDelta(thinking.to_string()))
                                            .ok();
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    "content_block_stop" => {}
                    "message_delta" => {
                        if let Some(delta) = parsed.get("delta") {
                            stop_reason = delta
                                .get("stop_reason")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                        }
                        if let Some(u) = parsed.get("usage") {
                            total_usage.output_tokens =
                                u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                        }
                    }
                    "message_stop" | "ping" => {}
                    "error" => {
                        if let Some(err) = parsed.get("error") {
                            let msg = err
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown error");
                            tx.send(StreamEvent::Error(msg.to_string())).ok();
                        }
                    }
                    _ => {}
                }
            }
        }

        // Build assistant message
        let content: Vec<ContentBlock> = blocks
            .iter()
            .filter_map(|b| match b.kind {
                BKind::Text if !b.text.is_empty() => Some(ContentBlock::Text {
                    text: b.text.clone(),
                }),
                BKind::ToolUse => {
                    let input: serde_json::Value =
                        serde_json::from_str(&b.text).unwrap_or(serde_json::Value::Null);
                    Some(ContentBlock::ToolUse {
                        id: b.tool_id.clone().unwrap_or_default(),
                        name: b.tool_name.clone().unwrap_or_default(),
                        input,
                    })
                }
                BKind::Thinking if !b.text.is_empty() => Some(ContentBlock::Thinking {
                    thinking: b.text.clone(),
                }),
                _ => None,
            })
            .collect();

        let msg = AssistantMessage {
            uuid: uuid::Uuid::new_v4().to_string(),
            content,
            timestamp: chrono::Utc::now().timestamp() as u64,
            model: Some(model),
            stop_reason,
            usage: Some(total_usage),
        };

        tx.send(StreamEvent::MessageComplete(msg.clone())).ok();

        Ok(msg)
    }
}

/// Build the system prompt from context.
async fn build_system_prompt(working_dir: &std::path::Path) -> String {
    let mut parts = Vec::new();

    parts.push("You are Claude, an AI assistant created by Anthropic. You help users with software engineering tasks.".to_string());

    // Add git context
    if let Ok(Some(git_ctx)) = git::get_git_context(working_dir).await {
        parts.push(format!(
            "\n# Git Context\nBranch: {}\nStatus:\n{}\nRecent commits:\n{}",
            git_ctx.branch, git_ctx.status, git_ctx.recent_log
        ));
    }

    // Add CLAUDE.md content
    let git_root = git::find_git_root(working_dir).await.ok().flatten();
    if let Ok(claude_files) = claudemd::load_claude_md_files(git_root.as_deref()).await {
        let merged = claudemd::merge_claude_md(&claude_files);
        if !merged.is_empty() {
            parts.push(format!("\n# Instructions\n{}", merged));
        }
    }

    parts.push(format!(
        "\n# Environment\nWorking directory: {}\nPlatform: {}\nDate: {}",
        working_dir.display(),
        std::env::consts::OS,
        chrono::Local::now().format("%Y-%m-%d"),
    ));

    parts.join("\n")
}

// Inline SSE parser to avoid circular dependency
struct SseParserInline {
    buffer: String,
}

impl SseParserInline {
    fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    fn next_raw_event(&mut self) -> Option<(String, String)> {
        let sep = if self.buffer.contains("\n\n") {
            "\n\n"
        } else if self.buffer.contains("\r\n\r\n") {
            "\r\n\r\n"
        } else {
            return None;
        };

        let pos = self.buffer.find(sep)?;
        let raw: String = self.buffer.drain(..pos + sep.len()).collect();

        let mut event_type = String::new();
        let mut data_lines = Vec::new();

        for line in raw.lines() {
            if let Some(v) = line.strip_prefix("event: ") {
                event_type = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("data: ") {
                data_lines.push(v.to_string());
            } else if let Some(v) = line.strip_prefix("data:") {
                data_lines.push(v.to_string());
            }
        }

        let data = data_lines.join("\n");
        Some((event_type, data))
    }
}

#[derive(Debug, Clone, Default)]
enum BKind {
    #[default]
    Text,
    ToolUse,
    Thinking,
}

#[derive(Debug, Clone, Default)]
struct BlockBuilder {
    kind: BKind,
    text: String,
    tool_id: Option<String>,
    tool_name: Option<String>,
}
