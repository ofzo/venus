use anyhow::{Context, Result};
use venus_core::message::{AssistantMessage, ContentBlock};
use venus_core::stream::StreamEvent;
use venus_utils::cost::TokenUsage;
use tracing::{debug, warn};

use crate::models::*;
use crate::sse::SseParser;
use futures_util::StreamExt;

pub struct AnthropicClient {
    api_key: String,
    base_url: String,
    http: reqwest::Client,
}

impl AnthropicClient {
    pub fn new(api_key: String, base_url: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("failed to build HTTP client");

        Self {
            api_key,
            base_url,
            http,
        }
    }

    /// Send a streaming message request and return events via callback.
    pub async fn create_message_stream(
        &self,
        request: CreateMessageRequest,
        mut on_event: impl FnMut(StreamEvent),
    ) -> Result<()> {
        let url = format!("{}/v1/messages", self.base_url);
        debug!("POST {} model={}", url, request.model);

        let response = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .context("failed to send API request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            if status.as_u16() == 429 || status.as_u16() == 529 {
                return Err(anyhow::anyhow!(
                    "rate limited ({}): {}",
                    status,
                    &body[..body.len().min(500)]
                ));
            }
            return Err(anyhow::anyhow!(
                "API error ({}): {}",
                status,
                &body[..body.len().min(500)]
            ));
        }

        // Parse SSE stream
        let byte_stream = response.bytes_stream();
        let mut parser = SseParser::new();
        let mut pinned = std::pin::pin!(byte_stream);

        // State for building the assistant message
        let mut model = String::new();
        let mut content_blocks: Vec<ContentBlockBuilder> = Vec::new();
        let mut stop_reason: Option<String> = None;
        let mut total_usage = TokenUsage::default();

        while let Some(chunk) = pinned.next().await {
            let chunk = chunk.context("stream read error")?;
            for event_result in parser.feed(&chunk) {
                let sse_event = match event_result {
                    Ok(e) => e,
                    Err(e) => {
                        warn!("SSE parse error: {}", e);
                        continue;
                    }
                };

                match sse_event {
                    SseEvent::MessageStart { message } => {
                        model = message.model;
                        if let Some(usage) = message.usage {
                            total_usage.input_tokens = usage.input_tokens;
                            total_usage.cache_read_tokens = usage.cache_read_input_tokens;
                            total_usage.cache_creation_tokens = usage.cache_creation_input_tokens;
                        }
                    }
                    SseEvent::ContentBlockStart {
                        index,
                        content_block,
                    } => {
                        while content_blocks.len() <= index {
                            content_blocks.push(ContentBlockBuilder::default());
                        }
                        match content_block {
                            ContentBlockData::Text { .. } => {
                                content_blocks[index].kind = BlockKind::Text;
                            }
                            ContentBlockData::ToolUse { id, name } => {
                                content_blocks[index].kind = BlockKind::ToolUse;
                                content_blocks[index].tool_id = Some(id.clone());
                                content_blocks[index].tool_name = Some(name.clone());
                                on_event(StreamEvent::ToolUseStart { id, name });
                            }
                            ContentBlockData::Thinking { signature, .. } => {
                                content_blocks[index].kind = BlockKind::Thinking;
                                content_blocks[index].signature = if signature.is_empty() { None } else { Some(signature) };
                            }
                        }
                    }
                    SseEvent::ContentBlockDelta { index, delta } => {
                        if index < content_blocks.len() {
                            match delta {
                                DeltaData::TextDelta { text } => {
                                    content_blocks[index].text.push_str(&text);
                                    on_event(StreamEvent::TextDelta(text));
                                }
                                DeltaData::InputJsonDelta { partial_json } => {
                                    content_blocks[index].text.push_str(&partial_json);
                                    on_event(StreamEvent::ToolUseInput(partial_json));
                                }
                                DeltaData::ThinkingDelta { thinking } => {
                                    content_blocks[index].text.push_str(&thinking);
                                    on_event(StreamEvent::ThinkingDelta(thinking));
                                }
                            }
                        }
                    }
                    SseEvent::ContentBlockStop { .. } => {}
                    SseEvent::MessageDelta { delta, usage } => {
                        stop_reason = delta.stop_reason;
                        if let Some(u) = usage {
                            total_usage.output_tokens = u.output_tokens;
                        }
                    }
                    SseEvent::MessageStop => {
                        // Build the final assistant message
                        let content: Vec<ContentBlock> = content_blocks
                            .iter()
                            .filter_map(|b| b.to_content_block())
                            .collect();

                        let msg = AssistantMessage {
                            uuid: uuid::Uuid::new_v4().to_string(),
                            content,
                            timestamp: chrono::Utc::now().timestamp() as u64,
                            model: Some(model.clone()),
                            stop_reason: stop_reason.clone(),
                            usage: Some(total_usage.clone()),
                        };

                        on_event(StreamEvent::Usage(total_usage.clone()));
                        on_event(StreamEvent::MessageComplete(msg));
                    }
                    SseEvent::Ping => {}
                    SseEvent::Error { error } => {
                        on_event(StreamEvent::Error(format!(
                            "{}: {}",
                            error.error_type, error.message
                        )));
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
enum BlockKind {
    #[default]
    Text,
    ToolUse,
    Thinking,
}

#[derive(Debug, Clone, Default)]
struct ContentBlockBuilder {
    kind: BlockKind,
    text: String,
    tool_id: Option<String>,
    tool_name: Option<String>,
    signature: Option<String>,
}

impl ContentBlockBuilder {
    fn to_content_block(&self) -> Option<ContentBlock> {
        match self.kind {
            BlockKind::Text => {
                if self.text.is_empty() {
                    None
                } else {
                    Some(ContentBlock::Text {
                        text: self.text.clone(),
                    })
                }
            }
            BlockKind::ToolUse => {
                let input: serde_json::Value =
                    serde_json::from_str(&self.text).unwrap_or(serde_json::Value::Null);
                Some(ContentBlock::ToolUse {
                    id: self.tool_id.clone().unwrap_or_default(),
                    name: self.tool_name.clone().unwrap_or_default(),
                    input,
                })
            }
            BlockKind::Thinking => {
                if self.text.is_empty() {
                    None
                } else {
                    Some(ContentBlock::Thinking {
                        thinking: self.text.clone(),
                        signature: self.signature.clone().unwrap_or_default(),
                    })
                }
            }
        }
    }
}
