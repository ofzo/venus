use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};
use venus_utils::cost::TokenUsage;

use crate::background::BackgroundTaskRuntime;
use venus_utils::claudemd;
use venus_utils::config::Settings;
use venus_utils::context_window;
use venus_utils::cost::CostTracker;
use venus_utils::git;
use venus_utils::venusmd;

use crate::hooks::HookRunner;
use crate::message::*;
use crate::stream::StreamEvent;
use crate::task::TaskStore;
use crate::tool::{PermissionDecision, PermissionHandler, ToolContext, ToolResult};
use crate::tool_registry::ToolRegistry;

#[derive(Clone)]
pub struct QueryEngine {
    pub session_id: String,
    /// Optional display name for the session.
    pub session_name: Option<String>,
    /// Prompt color (ANSI color name: blue, green, red, yellow, cyan, magenta, white).
    pub prompt_color: String,
    /// Terminal theme (dark, light, auto).
    pub theme: String,
    /// Auth header name: "Authorization" for Bearer tokens, "x-api-key" for API keys.
    pub auth_header: &'static str,
    /// Auth header value: "Bearer <token>" or raw API key.
    pub auth_value: String,
    pub model: String,
    pub base_url: String,
    pub max_tokens: u32,
    pub messages: Arc<tokio::sync::Mutex<Vec<Message>>>,
    pub tools: Arc<ToolRegistry>,
    pub settings: Arc<Settings>,
    pub permissions: Arc<dyn PermissionHandler>,
    pub cost_tracker: Arc<std::sync::Mutex<CostTracker>>,
    pub cancel_token: CancellationToken,
    pub working_dir: PathBuf,
    /// Additional working directories the engine can access.
    pub additional_working_dirs: Vec<PathBuf>,
    pub system_prompt: String,
    pub task_store: Arc<TaskStore>,
    pub background_runtime: Arc<BackgroundTaskRuntime>,
    pub created_at: u64,
    /// Counter for consecutive auto-compact failures (circuit breaker).
    pub auto_compact_failures: Arc<AtomicU32>,
    /// Hook runner for lifecycle event hooks.
    pub hook_runner: Arc<HookRunner>,
    /// Whether the engine is currently in plan mode.
    pub plan_mode: Arc<AtomicBool>,
    /// Optional cron scheduler for scheduled tasks.
    pub cron_scheduler: Option<Arc<crate::cron::CronScheduler>>,
    /// Maximum agentic turns per query.
    pub max_turns: u32,
    /// Maximum budget in USD (None = unlimited).
    pub budget_usd: Option<f64>,
}

impl QueryEngine {
    pub async fn new(
        settings: Arc<Settings>,
        tools: Arc<ToolRegistry>,
        permissions: Arc<dyn PermissionHandler>,
        working_dir: PathBuf,
        task_store: Arc<TaskStore>,
        background_runtime: Arc<BackgroundTaskRuntime>,
        hook_runner: Arc<HookRunner>,
    ) -> Result<Self> {
        let (auth_header, auth_value) = settings
            .resolve_auth()
            .context("API credential required: set VENUS_API_KEY, VENUS_AUTH_TOKEN, or configure in settings")?;

        let model = settings.effective_model().to_string();
        let base_url = settings.effective_base_url().to_string();
        let max_tokens = settings.effective_max_tokens();

        // Read max_turns and budget from settings
        let max_turns = settings.max_turns.unwrap_or(25);
        let budget_usd = settings.budget_usd;

        // Build system prompt and append custom_system_prompt if configured
        let mut system_prompt = build_system_prompt(&working_dir).await;
        if let Some(ref custom) = settings.custom_system_prompt {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(custom);
        }

        Ok(Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            session_name: None,
            prompt_color: "cyan".to_string(),
            theme: "dark".to_string(),
            auth_header,
            auth_value,
            model,
            base_url,
            max_tokens,
            messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            tools,
            settings,
            permissions,
            cost_tracker: Arc::new(std::sync::Mutex::new(CostTracker::new())),
            cancel_token: CancellationToken::new(),
            working_dir,
            additional_working_dirs: Vec::new(),
            system_prompt,
            task_store,
            background_runtime,
            created_at: chrono::Utc::now().timestamp() as u64,
            auto_compact_failures: Arc::new(AtomicU32::new(0)),
            hook_runner,
            plan_mode: Arc::new(AtomicBool::new(false)),
            cron_scheduler: None,
            max_turns,
            budget_usd,
        })
    }

    /// Create a QueryEngine for a sub-agent with a custom system prompt.
    /// Skips the expensive build_system_prompt (git context, CLAUDE.md loading).
    #[allow(clippy::too_many_arguments)]
    pub fn new_for_subagent(
        auth_header: &'static str,
        auth_value: String,
        model: String,
        base_url: String,
        max_tokens: u32,
        system_prompt: String,
        tools: Arc<ToolRegistry>,
        settings: Arc<Settings>,
        permissions: Arc<dyn PermissionHandler>,
        working_dir: PathBuf,
        task_store: Arc<TaskStore>,
        background_runtime: Arc<BackgroundTaskRuntime>,
        hook_runner: Arc<HookRunner>,
        parent_cost_tracker: Option<Arc<std::sync::Mutex<venus_utils::cost::CostTracker>>>,
    ) -> Self {
        // A sub-agent contributes its token usage to the *parent* engine's
        // cost tracker when one is supplied, so the true total (main + any
        // sub-agents) becomes visible at exit-time / in the status bar. Fall
        // back to an isolated tracker for unit tests that pass None.
        let cost_tracker = parent_cost_tracker
            .unwrap_or_else(|| Arc::new(std::sync::Mutex::new(venus_utils::cost::CostTracker::new())));
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            session_name: None,
            prompt_color: "cyan".to_string(),
            theme: "dark".to_string(),
            auth_header,
            auth_value,
            model,
            base_url,
            max_tokens,
            messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            tools,
            settings,
            permissions,
            cost_tracker,
            cancel_token: CancellationToken::new(),
            working_dir,
            additional_working_dirs: Vec::new(),
            system_prompt,
            task_store,
            background_runtime,
            created_at: chrono::Utc::now().timestamp() as u64,
            auto_compact_failures: Arc::new(AtomicU32::new(0)),
            hook_runner,
            plan_mode: Arc::new(AtomicBool::new(false)),
            cron_scheduler: None,
            max_turns: 25,
            budget_usd: None,
        }
    }

    /// Submit a user message and spawn the query loop in a background task.
    /// Returns a receiver that streams events in real-time.
    pub async fn submit_message(
        &self,
        content: Vec<ContentBlock>,
    ) -> Result<mpsc::UnboundedReceiver<StreamEvent>> {
        let user_msg = UserMessage::new(content);
        self.messages.lock().await.push(Message::User(user_msg));

        let (tx, rx) = mpsc::unbounded_channel();

        // Clone the engine for the spawned task. The Arc-based fields are
        // shared with the original engine, so messages/cost_tracker updates
        // are visible to both.
        let engine = self.clone();

        tokio::spawn(async move {
            if let Err(e) = engine.run_query_loop(tx.clone()).await {
                tx.send(StreamEvent::Error(e.to_string())).ok();
            }
            // tx is dropped here, closing the channel and signaling completion
        });

        Ok(rx)
    }

    async fn run_query_loop(&self, tx: mpsc::UnboundedSender<StreamEvent>) -> Result<()> {
        let max_iterations = self.max_turns;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        let url = format!("{}/v1/messages", self.base_url);

        for iteration in 0..max_iterations {
            debug!("query loop iteration {}", iteration);

            // Build API request
            let request = self.build_api_request_body().await;
            let request_body = serde_json::to_string(&request)?;

            // Make streaming API call with retry (covers both connection and mid-stream failures)
            let assistant_msg = self
                .stream_with_retry(&client, &url, &request_body, &tx)
                .await?;

            let stop_reason = assistant_msg.stop_reason.clone();

            // Record usage
            if let Some(usage) = &assistant_msg.usage {
                self.cost_tracker.lock().unwrap().record(&self.model, usage);
                tx.send(StreamEvent::Usage(usage.clone())).ok();

                // Check budget limit
                if let Some(budget) = self.budget_usd {
                    let total_cost = self.cost_tracker.lock().unwrap().total_cost_usd();
                    if total_cost >= budget {
                        tx.send(StreamEvent::Error(format!(
                            "Budget limit reached: ${:.2} >= ${:.2}",
                            total_cost, budget
                        )))
                        .ok();
                        break;
                    }
                }

                // Check if auto-compact should trigger
                let current_input_tokens = usage.input_tokens + usage.cache_read_tokens;
                let threshold = venus_utils::context_window::auto_compact_threshold(&self.model);

                if current_input_tokens >= threshold {
                    let config = crate::compact::CompactConfig::from_engine(
                        &self.model,
                        self.auth_header,
                        &self.auth_value,
                        &self.base_url,
                    );
                    let mut messages = self.messages.lock().await;
                    let mut failures = self.auto_compact_failures.load(Ordering::Relaxed);
                    if let Ok(Some(result)) =
                        crate::compact::auto_compact(&mut messages, &config, &mut failures).await
                    {
                        self.auto_compact_failures
                            .store(failures, Ordering::Relaxed);
                        drop(messages);
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
            self.messages
                .lock()
                .await
                .push(Message::Assistant(assistant_msg));

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
            {
                let mut messages = self.messages.lock().await;
                for ((id, _name, _input), result) in tool_calls.iter().zip(results) {
                    let tool_result_msg =
                        Message::User(UserMessage::new(vec![ContentBlock::tool_result(
                            id.clone(),
                            result.content.clone(),
                            result.is_error,
                        )]));
                    messages.push(tool_result_msg);
                }
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
                let reason = hook_resp.reason.unwrap_or_else(|| "blocked by hook".into());
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
        let decision = self
            .permissions
            .check_permission(name, &effective_input)
            .await;
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
            background_runtime: self.background_runtime.clone(),
            plan_mode: self.plan_mode.clone(),
            messages: self.messages.clone(),
            auth_header: self.auth_header,
            auth_value: self.auth_value.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            tools: self.tools.clone(),
            hook_runner: self.hook_runner.clone(),
            cron_scheduler: self.cron_scheduler.clone(),
            cost_tracker: Some(self.cost_tracker.clone()),
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
        use futures_util::StreamExt;

        let mut parser = SseParserInline::new();
        let byte_stream = response.bytes_stream();
        let mut pinned = std::pin::pin!(byte_stream);

        let mut model = String::new();
        let mut blocks: Vec<BlockBuilder> = Vec::new();
        let mut stop_reason: Option<String> = None;
        let mut total_usage = TokenUsage::default();

        loop {
            // Check for cancellation before each chunk
            if self.cancel_token.is_cancelled() {
                debug!("stream cancelled by user");
                break;
            }

            // Race between the next chunk and cancellation
            let chunk_result = tokio::select! {
                chunk = pinned.next() => chunk,
                _ = self.cancel_token.cancelled() => {
                    debug!("stream cancelled during await");
                    break;
                }
            };

            let chunk = match chunk_result {
                Some(Ok(c)) => c,
                Some(Err(e)) => {
                    // Build checkpoint from current state
                    let checkpoint_blocks: Vec<ContentBlock> =
                        blocks.iter().filter_map(|b| b.to_content_block()).collect();
                    let has_content = checkpoint_blocks
                        .iter()
                        .any(|b| matches!(b, ContentBlock::Text { text } if !text.is_empty()));

                    return Err(StreamRecoveryError {
                        checkpoint: StreamCheckpoint {
                            blocks: checkpoint_blocks,
                            model: model.clone(),
                            input_usage: total_usage.clone(),
                            has_content,
                        },
                        message: e.to_string(),
                    }
                    .into());
                }
                None => break, // Stream ended normally
            };
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
        let content: Vec<ContentBlock> =
            blocks.iter().filter_map(|b| b.to_content_block()).collect();

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
    /// When partial content has been received before a failure, attempts checkpoint
    /// recovery by injecting the partial assistant response into the conversation
    /// and requesting continuation.
    async fn stream_with_retry(
        &self,
        client: &reqwest::Client,
        url: &str,
        body: &str,
        tx: &mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<AssistantMessage> {
        const STREAM_RETRIES: u32 = 2;

        for attempt in 0..=STREAM_RETRIES {
            let response = self.send_with_retry(client, url, body, tx).await?;

            match self.process_sse_stream(response, tx).await {
                Ok(msg) => return Ok(msg),
                Err(e) => {
                    // Try to extract checkpoint from error for smart recovery
                    if let Some(recovery_err) = e.downcast_ref::<StreamRecoveryError>() {
                        if recovery_err.checkpoint.has_content && attempt < STREAM_RETRIES {
                            debug!(
                                "stream interrupted with {} partial blocks, attempting recovery (attempt {}/{})",
                                recovery_err.checkpoint.blocks.len(),
                                attempt + 1,
                                STREAM_RETRIES
                            );
                            tx.send(StreamEvent::Error(
                                "Stream interrupted, recovering...".into(),
                            ))
                            .ok();

                            // Clone the checkpoint before the borrow on `e` is released
                            let checkpoint = recovery_err.checkpoint.clone();

                            // Attempt recovery with checkpoint
                            return self
                                .recover_from_checkpoint(&checkpoint, client, url, tx)
                                .await;
                        }
                    }

                    // No checkpoint or no content -- retry from scratch (existing behavior)
                    if attempt < STREAM_RETRIES {
                        debug!(
                            "stream failed, retrying from scratch (attempt {}/{}): {}",
                            attempt + 1,
                            STREAM_RETRIES,
                            e
                        );
                        tx.send(StreamEvent::Error(
                            "Stream interrupted, reconnecting...".to_string(),
                        ))
                        .ok();
                        let delay = 1000u64 * 2u64.pow(attempt);
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    } else {
                        return Err(e).context("stream processing failed after retries");
                    }
                }
            }
        }

        Err(anyhow::anyhow!("stream retries exhausted"))
    }

    /// Recover from a mid-stream interruption by saving partial output and requesting continuation.
    async fn recover_from_checkpoint(
        &self,
        checkpoint: &StreamCheckpoint,
        client: &reqwest::Client,
        url: &str,
        tx: &mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<AssistantMessage> {
        // Filter to only text and thinking blocks (skip incomplete tool_use)
        let partial_content: Vec<ContentBlock> = checkpoint
            .blocks
            .iter()
            .filter(|b| {
                matches!(b, ContentBlock::Text { text } if !text.is_empty())
                    || matches!(b, ContentBlock::Thinking { .. })
            })
            .cloned()
            .collect();

        if partial_content.is_empty() {
            anyhow::bail!("no recoverable content in checkpoint");
        }

        // 1. Add partial assistant message to conversation history
        let partial_msg = AssistantMessage {
            uuid: uuid::Uuid::new_v4().to_string(),
            content: partial_content,
            timestamp: chrono::Utc::now().timestamp() as u64,
            model: Some(checkpoint.model.clone()),
            stop_reason: None,
            usage: Some(checkpoint.input_usage.clone()),
        };

        {
            let mut messages = self.messages.lock().await;
            messages.push(Message::Assistant(partial_msg));

            // 2. Add continuation user message
            messages.push(Message::User(UserMessage::new(vec![
                ContentBlock::text(
                    "[The previous response was interrupted mid-stream. Continue exactly where you left off.]",
                ),
            ])));
        }

        // 3. Build a fresh API request body with the updated conversation
        let recovery_body = self.build_api_request_body().await;
        let recovery_body_str = serde_json::to_string(&recovery_body)?;

        // 4. Send and process without recovery (to avoid infinite recursion)
        let response = self
            .send_with_retry(client, url, &recovery_body_str, tx)
            .await?;
        self.process_sse_stream(response, tx).await
    }

    /// Build the full API request body from the current engine state.
    /// Extracted as a helper to allow reuse in recovery paths.
    async fn build_api_request_body(&self) -> serde_json::Value {
        let messages = self.messages.lock().await;
        let api_messages = messages_to_api_params(&messages);
        let mut tool_defs = self.tools.api_definitions();

        // Build system prompt as array with cache_control
        let system_blocks = serde_json::json!([{
            "type": "text",
            "text": self.system_prompt,
            "cache_control": {"type": "ephemeral"}
        }]);

        // Add cache_control to the last tool definition
        if let Some(last) = tool_defs.last_mut() {
            if let Some(obj) = last.as_object_mut() {
                obj.insert(
                    "cache_control".into(),
                    serde_json::json!({"type": "ephemeral"}),
                );
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
            request
                .as_object_mut()
                .unwrap()
                .insert("thinking".into(), thinking);
        }

        request
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
                .header("VENUS-version", "2023-06-01")
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .await;

            let response = match result {
                Ok(r) => r,
                Err(e) if attempt < MAX_RETRIES && e.is_timeout() => {
                    let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                    debug!(
                        "request timeout, retrying in {}ms (attempt {}/{})",
                        delay,
                        attempt + 1,
                        MAX_RETRIES
                    );
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
                let err_msg = format!(
                    "API error ({}): {}",
                    status,
                    &body_text[..body_text.len().min(500)]
                );
                // Do NOT push a `StreamEvent::Error` here: this error is
                // terminal and propagates via the `Err` return up to the
                // query-loop spawn boundary (`submit_message`), which is
                // the single source of truth for surfacing terminal errors.
                // Emitting here too produced a duplicated error line in the
                // transcript (one inner, one outer).
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
                status_code,
                delay,
                attempt + 1,
                MAX_RETRIES
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

    parts.push(
        "You are Venus, an AI coding assistant. You help users with software engineering tasks."
            .to_string(),
    );

    // Add git context
    if let Ok(Some(git_ctx)) = git::get_git_context(working_dir).await {
        parts.push(format!(
            "\n# Git Context\nBranch: {}\nStatus:\n{}\nRecent commits:\n{}",
            git_ctx.branch, git_ctx.status, git_ctx.recent_log
        ));
    }

    // Add VENUS.md content
    let git_root = git::find_git_root(working_dir).await.ok().flatten();
    if let Ok(venus_files) = venusmd::load_venus_md_files(git_root.as_deref()).await {
        let merged = venusmd::merge_venus_md(&venus_files);
        if !merged.is_empty() {
            parts.push(format!("\n# Instructions\n{}", merged));
        }
    }

    // Add CLAUDE.md content (legacy compatibility)
    if let Ok(claude_files) = claudemd::load_claude_md_files(git_root.as_deref()).await {
        let merged = claudemd::merge_claude_md(&claude_files);
        if !merged.is_empty() {
            parts.push(format!("\n# CLAUDE.md Instructions\n{}", merged));
        }
    }

    // Load memory content
    if let Ok(memory_content) = venus_utils::memory::load_memory_for_prompt(Some(working_dir)).await
    {
        if !memory_content.is_empty() {
            parts.push(format!("\n# Memory\n{}", memory_content));
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

impl BlockBuilder {
    /// Convert this builder into a ContentBlock, if it has meaningful content.
    fn to_content_block(&self) -> Option<ContentBlock> {
        match self.kind {
            BKind::Text if !self.text.is_empty() => Some(ContentBlock::Text {
                text: self.text.clone(),
            }),
            BKind::ToolUse => {
                let input: serde_json::Value =
                    serde_json::from_str(&self.text).unwrap_or(serde_json::Value::Null);
                Some(ContentBlock::ToolUse {
                    id: self.tool_id.clone().unwrap_or_default(),
                    name: self.tool_name.clone().unwrap_or_default(),
                    input,
                })
            }
            BKind::Thinking if !self.text.is_empty() => Some(ContentBlock::Thinking {
                thinking: self.text.clone(),
                signature: self.signature.clone().unwrap_or_default(),
            }),
            _ => None,
        }
    }
}

/// Checkpoint state for stream recovery. Captures accumulated content
/// so that a mid-stream failure can be recovered without losing partial output.
#[derive(Debug, Clone, Default)]
struct StreamCheckpoint {
    /// Content blocks accumulated before the interruption.
    blocks: Vec<ContentBlock>,
    /// Model name from message_start event.
    model: String,
    /// Input token usage from message_start (for cost tracking).
    input_usage: TokenUsage,
    /// True if at least one non-empty content block was accumulated.
    has_content: bool,
}

/// Error type for stream interruptions that carries a recovery checkpoint.
#[derive(Debug)]
struct StreamRecoveryError {
    checkpoint: StreamCheckpoint,
    message: String,
}

impl std::fmt::Display for StreamRecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "stream interrupted: {}", self.message)
    }
}

impl std::error::Error for StreamRecoveryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::BackgroundTaskRuntime;
    use crate::hooks::HookRunner;
    use crate::task::TaskStore;
    use crate::tool_registry::ToolRegistry;
    use std::path::PathBuf;
    use std::sync::Arc;
    use venus_utils::config::Settings;

    /// A minimal permission handler for testing that allows all tools.
    struct AllowAllPermissions;

    #[async_trait::async_trait]
    impl PermissionHandler for AllowAllPermissions {
        async fn check_permission(
            &self,
            _tool_name: &str,
            _input: &serde_json::Value,
        ) -> PermissionDecision {
            PermissionDecision::Allow
        }
    }

    fn test_settings() -> Arc<Settings> {
        use std::collections::HashMap;
        use venus_utils::config::ProviderConfig;
        let mut providers = HashMap::new();
        providers.insert(
            "VENUS".to_string(),
            ProviderConfig {
                provider_type: "VENUS".to_string(),
                api_key: Some("test-key".to_string()),
                auth_token: None,
                base_url: None,
                default_model: None,
                max_tokens: None,
                api_version: None,
            },
        );
        Arc::new(Settings {
            active_provider: Some("VENUS".to_string()),
            provider: Some(providers),
            ..Default::default()
        })
    }

    fn test_engine_parts() -> (
        Arc<ToolRegistry>,
        Arc<dyn PermissionHandler>,
        Arc<TaskStore>,
        Arc<BackgroundTaskRuntime>,
        Arc<HookRunner>,
    ) {
        let tools = Arc::new(ToolRegistry::new(vec![]));
        let permissions: Arc<dyn PermissionHandler> = Arc::new(AllowAllPermissions);
        let task_store = Arc::new(TaskStore::new());
        let background_runtime = Arc::new(BackgroundTaskRuntime::new());
        let hook_runner = Arc::new(HookRunner::new(
            None,
            "test-session".to_string(),
            PathBuf::from("/tmp"),
        ));
        (
            tools,
            permissions,
            task_store,
            background_runtime,
            hook_runner,
        )
    }

    #[tokio::test]
    async fn test_engine_creation() {
        let settings = test_settings();
        let (tools, permissions, task_store, bg, hooks) = test_engine_parts();
        let engine = QueryEngine::new(
            settings,
            tools,
            permissions,
            PathBuf::from("/tmp"),
            task_store,
            bg,
            hooks,
        )
        .await;
        assert!(engine.is_ok());
        let engine = engine.unwrap();
        assert_eq!(engine.max_turns, 25);
        assert_eq!(engine.budget_usd, None);
    }

    #[tokio::test]
    async fn test_submit_message_returns_receiver() {
        let settings = test_settings();
        let (tools, permissions, task_store, bg, hooks) = test_engine_parts();
        let engine = QueryEngine::new(
            settings,
            tools,
            permissions,
            PathBuf::from("/tmp"),
            task_store,
            bg,
            hooks,
        )
        .await
        .unwrap();

        // submit_message should return a receiver immediately
        // (it spawns the query loop in the background)
        let result = engine
            .submit_message(vec![ContentBlock::text("hello")])
            .await;
        assert!(result.is_ok());

        let rx = result.unwrap();
        // The spawned task will fail because there's no real API endpoint,
        // but the receiver should still be returned. We'll get an Error event.
        // Give the spawned task a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // The receiver should have at least one event (either an error from the
        // failed API call, or possibly nothing if the task hasn't started yet)
        // Just verify the receiver exists and is usable
        drop(rx);
    }

    #[tokio::test]
    async fn test_submit_message_pushes_user_message() {
        let settings = test_settings();
        let (tools, permissions, task_store, bg, hooks) = test_engine_parts();
        let engine = QueryEngine::new(
            settings,
            tools,
            permissions,
            PathBuf::from("/tmp"),
            task_store,
            bg,
            hooks,
        )
        .await
        .unwrap();

        engine
            .submit_message(vec![ContentBlock::text("test message")])
            .await
            .unwrap();

        // The user message should be in the shared messages
        let messages = engine.messages.lock().await;
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            Message::User(user_msg) => {
                assert!(!user_msg.content.is_empty());
            }
            _ => panic!("Expected user message"),
        }
    }

    #[tokio::test]
    async fn test_shared_messages_between_engine_and_clone() {
        let settings = test_settings();
        let (tools, permissions, task_store, bg, hooks) = test_engine_parts();
        let engine = QueryEngine::new(
            settings,
            tools,
            permissions,
            PathBuf::from("/tmp"),
            task_store,
            bg,
            hooks,
        )
        .await
        .unwrap();

        // Clone shares the same messages Arc
        let clone = engine.clone();
        engine
            .submit_message(vec![ContentBlock::text("shared test")])
            .await
            .unwrap();

        // Clone should see the same messages
        let messages = clone.messages.lock().await;
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_default_max_turns() {
        let settings = test_settings();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tools, permissions, task_store, bg, hooks) = test_engine_parts();
        let engine = rt.block_on(QueryEngine::new(
            settings,
            tools,
            permissions,
            PathBuf::from("/tmp"),
            task_store,
            bg,
            hooks,
        ));
        assert!(engine.is_ok());
        assert_eq!(engine.unwrap().max_turns, 25);
    }

    #[test]
    fn test_budget_usd_default_none() {
        let settings = test_settings();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tools, permissions, task_store, bg, hooks) = test_engine_parts();
        let engine = rt.block_on(QueryEngine::new(
            settings,
            tools,
            permissions,
            PathBuf::from("/tmp"),
            task_store,
            bg,
            hooks,
        ));
        assert!(engine.is_ok());
        assert_eq!(engine.unwrap().budget_usd, None);
    }
}
