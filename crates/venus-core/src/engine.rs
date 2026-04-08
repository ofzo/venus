use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use venus_utils::claudemd;
use venus_utils::config::Settings;
use venus_utils::context_window;
use venus_utils::cost::CostTracker;
use venus_utils::git;

use crate::hooks::HookRunner;
use crate::message::*;
use crate::stream::StreamEvent;
use crate::task::TaskStore;
use crate::tool::{PermissionDecision, PermissionHandler, ToolContext, ToolResult};
use crate::tool_registry::ToolRegistry;

pub struct QueryEngine {
    pub session_id: String,
    /// Auth header name: "Authorization" for Bearer tokens, "x-api-key" for API keys.
    pub auth_header: &'static str,
    /// Auth header value: "Bearer <token>" or raw API key.
    pub auth_value: String,
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
    pub created_at: u64,
    /// Counter for consecutive auto-compact failures (circuit breaker).
    pub auto_compact_failures: u32,
    /// Hook runner for lifecycle event hooks.
    pub hook_runner: Arc<HookRunner>,
}

impl QueryEngine {
    pub async fn new(
        settings: Arc<Settings>,
        tools: Arc<ToolRegistry>,
        permissions: Arc<dyn PermissionHandler>,
        working_dir: PathBuf,
        task_store: Arc<TaskStore>,
        hook_runner: Arc<HookRunner>,
    ) -> Result<Self> {
        let (auth_header, auth_value) = settings
            .resolve_auth()
            .context("API credential required: set ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN, or configure in settings")?;

        let model = settings.effective_model().to_string();
        let base_url = settings.effective_base_url().to_string();
        let max_tokens = settings.effective_max_tokens();

        // Build system prompt
        let system_prompt = build_system_prompt(&working_dir).await;

        Ok(Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            auth_header,
            auth_value,
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
            created_at: chrono::Utc::now().timestamp() as u64,
            auto_compact_failures: 0,
            hook_runner,
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

            // Build system prompt as array with cache_control
            let system_blocks = serde_json::json!([{
                "type": "text",
                "text": self.system_prompt,
                "cache_control": {"type": "ephemeral"}
            }]);

            // Add cache_control to the last tool definition
            let mut tool_defs = tool_defs;
            if let Some(last) = tool_defs.last_mut() {
                if let Some(obj) = last.as_object_mut() {
                    obj.insert("cache_control".into(), serde_json::json!({"type": "ephemeral"}));
                }
            }

            // Add cache_control to last user messages for conversation turn caching
            let api_messages = add_cache_control_to_messages(api_messages);

            // Determine max_tokens: when thinking is enabled, use model's full max output
            let thinking_param = self.build_thinking_param();
            let max_tokens = if thinking_param.is_some() {
                context_window::max_output_for_model(&self.model)
            } else {
                self.max_tokens
            };

            let mut request = serde_json::json!({
                "model": self.model,
                "max_tokens": max_tokens,
                "system": system_blocks,
                "messages": api_messages,
                "tools": tool_defs,
                "stream": true,
            });

            if let Some(thinking) = thinking_param {
                request.as_object_mut().unwrap().insert("thinking".into(), thinking);
            }

            // Make streaming API call with retry (covers both connection and mid-stream failures)
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()?;

            let url = format!("{}/v1/messages", self.base_url);
            let request_body = serde_json::to_string(&request)?;

            let assistant_msg = self
                .stream_with_retry(&client, &url, &request_body, &tx)
                .await?;

            let stop_reason = assistant_msg.stop_reason.clone();

            // Record usage
            if let Some(usage) = &assistant_msg.usage {
                self.cost_tracker.record(&self.model, usage);
                tx.send(StreamEvent::Usage(usage.clone())).ok();

                // Check if auto-compact should trigger
                let current_input_tokens = usage.input_tokens + usage.cache_read_tokens;
                let threshold =
                    venus_utils::context_window::auto_compact_threshold(&self.model);

                if current_input_tokens >= threshold {
                    let config = crate::compact::CompactConfig::from_engine(
                        &self.model,
                        self.auth_header,
                        &self.auth_value,
                        &self.base_url,
                    );
                    if let Ok(Some(result)) = crate::compact::auto_compact(
                        &mut self.messages,
                        &config,
                        &mut self.auto_compact_failures,
                    )
                    .await
                    {
                        tx.send(StreamEvent::AutoCompacted {
                            messages_removed: result.messages_before - result.messages_after,
                            tokens_saved: result.tokens_saved_estimate,
                        })
                        .ok();
                    }
                }
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

            // Execute tool calls in parallel
            let tool_futures: Vec<_> = tool_calls
                .iter()
                .map(|(id, name, input)| self.execute_tool(id, name, input, &tx))
                .collect();

            let results = futures_util::future::join_all(tool_futures).await;

            // Add all tool results to messages
            for ((id, _name, _input), result) in tool_calls.iter().zip(results) {
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
        // Run PreToolUse hooks
        let mut effective_input = input.clone();
        if let Ok(hook_resp) = self.hook_runner.run_pre_tool_use(name, input).await {
            if hook_resp.decision.as_deref() == Some("deny") {
                let reason = hook_resp
                    .reason
                    .unwrap_or_else(|| "blocked by hook".into());
                let result = ToolResult::error(format!("Hook denied: {}", reason));
                tx.send(StreamEvent::ToolResult {
                    id: id.to_string(),
                    name: name.to_string(),
                    result: result.clone(),
                })
                .ok();
                return result;
            }
            if let Some(updated) = hook_resp.updated_input {
                effective_input = updated;
            }
        }

        // Check permission
        let decision = self.permissions.check_permission(name, &effective_input).await;
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
            session_id: self.session_id.clone(),
            cancel_token: self.cancel_token.clone(),
            permission_handler: self.permissions.clone(),
            settings: self.settings.clone(),
            task_store: self.task_store.clone(),
        };

        info!("executing tool: {} with input: {}", name, &effective_input);

        let result = match tool.execute(effective_input.clone(), &ctx).await {
            Ok(r) => r,
            Err(e) => ToolResult::error(format!("tool error: {}", e)),
        };

        tx.send(StreamEvent::ToolResult {
            id: id.to_string(),
            name: name.to_string(),
            result: result.clone(),
        })
        .ok();

        // Run PostToolUse hooks (non-blocking)
        let runner = self.hook_runner.clone();
        let tool_name = name.to_string();
        let tool_input = effective_input;
        let result_json =
            serde_json::to_string(&result.content).unwrap_or_else(|_| "[]".to_string());
        let is_err = result.is_error;
        tokio::spawn(async move {
            runner
                .run_post_tool_use(&tool_name, &tool_input, &result_json, is_err)
                .await;
        });

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
                                "thinking" => {
                                    blocks[index].kind = BKind::Thinking;
                                    blocks[index].signature = cb
                                        .get("signature")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                }
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
                    signature: b.signature.clone().unwrap_or_default(),
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

    /// Send request + process stream, retrying on mid-stream failures.
    async fn stream_with_retry(
        &self,
        client: &reqwest::Client,
        url: &str,
        body: &str,
        tx: &mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<AssistantMessage> {
        const STREAM_RETRIES: u32 = 2;

        for attempt in 0..=STREAM_RETRIES {
            let response = self
                .send_with_retry(client, url, body, tx)
                .await?;

            match self.process_sse_stream(response, tx).await {
                Ok(msg) => return Ok(msg),
                Err(e) if attempt < STREAM_RETRIES => {
                    debug!(
                        "stream interrupted, retrying (attempt {}/{}): {}",
                        attempt + 1, STREAM_RETRIES, e
                    );
                    tx.send(StreamEvent::Error(
                        "Stream interrupted, reconnecting...".to_string(),
                    ))
                    .ok();
                    let delay = 1000u64 * 2u64.pow(attempt);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
                Err(e) => return Err(e).context("stream processing failed after retries"),
            }
        }

        Err(anyhow::anyhow!("stream retries exhausted"))
    }

    /// Send an API request with exponential backoff retry for rate limits and server errors.
    async fn send_with_retry(
        &self,
        client: &reqwest::Client,
        url: &str,
        body: &str,
        tx: &mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<reqwest::Response> {
        const MAX_RETRIES: u32 = 5;
        const BASE_DELAY_MS: u64 = 1000;

        for attempt in 0..=MAX_RETRIES {
            let result = client
                .post(url)
                .header(self.auth_header, &self.auth_value)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .await;

            let response = match result {
                Ok(r) => r,
                Err(e) if attempt < MAX_RETRIES && e.is_timeout() => {
                    let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                    debug!("request timeout, retrying in {}ms (attempt {}/{})", delay, attempt + 1, MAX_RETRIES);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    continue;
                }
                Err(e) => return Err(e).context("API request failed"),
            };

            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }

            let status_code = status.as_u16();
            let is_retryable = status_code == 429 || status_code == 529 || status_code >= 500;

            if !is_retryable || attempt >= MAX_RETRIES {
                let body_text = response.text().await.unwrap_or_default();
                let err_msg = format!("API error ({}): {}", status, &body_text[..body_text.len().min(500)]);
                tx.send(StreamEvent::Error(err_msg.clone())).ok();
                return Err(anyhow::anyhow!(err_msg));
            }

            // Parse Retry-After header if present
            let retry_after_ms = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(|secs| secs * 1000);

            let delay = retry_after_ms.unwrap_or_else(|| BASE_DELAY_MS * 2u64.pow(attempt));
            let delay = delay.min(60_000); // cap at 60s

            debug!(
                "API returned {}, retrying in {}ms (attempt {}/{})",
                status_code, delay, attempt + 1, MAX_RETRIES
            );
            tx.send(StreamEvent::Error(format!(
                "Rate limited ({}), retrying in {:.1}s...",
                status_code,
                delay as f64 / 1000.0
            )))
            .ok();

            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }

        Err(anyhow::anyhow!("max retries exceeded"))
    }

    /// Build the thinking parameter for the API request, if applicable.
    fn build_thinking_param(&self) -> Option<serde_json::Value> {
        if std::env::var("CLAUDE_CODE_DISABLE_THINKING").is_ok() {
            return None;
        }
        if !context_window::model_supports_thinking(&self.model) {
            return None;
        }

        let thinking_config = self.settings.thinking.as_ref();
        if thinking_config.and_then(|t| t.mode.as_deref()) == Some("disabled") {
            return None;
        }

        if context_window::model_supports_adaptive_thinking(&self.model) {
            Some(serde_json::json!({"type": "adaptive"}))
        } else {
            let budget = thinking_config
                .and_then(|t| t.budget_tokens)
                .unwrap_or_else(|| context_window::max_thinking_budget(&self.model));
            Some(serde_json::json!({"type": "enabled", "budget_tokens": budget}))
        }
    }
}

/// Add cache_control to the last content block of the last 2 user messages.
fn add_cache_control_to_messages(mut messages: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let len = messages.len();
    let mut user_count = 0;
    for i in (0..len).rev() {
        if user_count >= 2 {
            break;
        }
        if messages[i].get("role").and_then(|v| v.as_str()) == Some("user") {
            if let Some(content) = messages[i].get_mut("content") {
                if let Some(arr) = content.as_array_mut() {
                    if let Some(last_block) = arr.last_mut() {
                        if let Some(obj) = last_block.as_object_mut() {
                            obj.insert(
                                "cache_control".into(),
                                serde_json::json!({"type": "ephemeral"}),
                            );
                        }
                    }
                }
            }
            user_count += 1;
        }
    }
    messages
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
            if let Some(v) = line.strip_prefix("event:") {
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
    signature: Option<String>,
}
