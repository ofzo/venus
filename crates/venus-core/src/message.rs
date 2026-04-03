use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    #[serde(rename = "user")]
    User(UserMessage),
    #[serde(rename = "assistant")]
    Assistant(AssistantMessage),
    #[serde(rename = "system")]
    System(SystemMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessage {
    pub uuid: String,
    pub content: Vec<ContentBlock>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub uuid: String,
    pub content: Vec<ContentBlock>,
    pub timestamp: u64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<venus_utils::cost::TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMessage {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: Vec<ContentBlock>,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

impl ContentBlock {
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }

    pub fn tool_result(tool_use_id: String, content: Vec<ContentBlock>, is_error: bool) -> Self {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        }
    }
}

impl UserMessage {
    pub fn new(content: Vec<ContentBlock>) -> Self {
        Self {
            uuid: uuid::Uuid::new_v4().to_string(),
            content,
            timestamp: chrono::Utc::now().timestamp() as u64,
        }
    }
}

impl AssistantMessage {
    pub fn new(content: Vec<ContentBlock>) -> Self {
        Self {
            uuid: uuid::Uuid::new_v4().to_string(),
            content,
            timestamp: chrono::Utc::now().timestamp() as u64,
            model: None,
            stop_reason: None,
            usage: None,
        }
    }
}

/// Convert messages to the API format expected by Anthropic Messages API.
pub fn messages_to_api_params(messages: &[Message]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter_map(|msg| match msg {
            Message::User(u) => Some(serde_json::json!({
                "role": "user",
                "content": content_blocks_to_api(&u.content),
            })),
            Message::Assistant(a) => Some(serde_json::json!({
                "role": "assistant",
                "content": content_blocks_to_api(&a.content),
            })),
            Message::System(_) => None,
        })
        .collect()
}

fn content_blocks_to_api(blocks: &[ContentBlock]) -> serde_json::Value {
    let api_blocks: Vec<serde_json::Value> = blocks
        .iter()
        .map(|b| match b {
            ContentBlock::Text { text } => serde_json::json!({
                "type": "text",
                "text": text,
            }),
            ContentBlock::ToolUse { id, name, input } => serde_json::json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input,
            }),
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content_blocks_to_api(content),
                "is_error": is_error,
            }),
            ContentBlock::Thinking { thinking } => serde_json::json!({
                "type": "thinking",
                "thinking": thinking,
            }),
        })
        .collect();

    serde_json::Value::Array(api_blocks)
}
