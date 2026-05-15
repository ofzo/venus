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
    Thinking { thinking: String, signature: String },
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

impl Message {
    /// Returns the timestamp of this message, if available.
    pub fn timestamp(&self) -> Option<u64> {
        match self {
            Message::User(m) => Some(m.timestamp),
            Message::Assistant(m) => Some(m.timestamp),
            Message::System(_) => None,
        }
    }

    /// Returns true if this is a user message containing a tool result.
    pub fn is_tool_result(&self) -> bool {
        match self {
            Message::User(m) => m
                .content
                .first()
                .map(|b| matches!(b, ContentBlock::ToolResult { .. }))
                .unwrap_or(false),
            _ => false,
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
            ContentBlock::Thinking { thinking, signature } => serde_json::json!({
                "type": "thinking",
                "thinking": thinking,
                "signature": signature,
            }),
        })
        .collect();

    serde_json::Value::Array(api_blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_block_text() {
        let block = ContentBlock::text("hello");
        match block {
            ContentBlock::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_content_block_tool_result() {
        let block = ContentBlock::tool_result(
            "tool_1".to_string(),
            vec![ContentBlock::text("result")],
            false,
        );
        match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "tool_1");
                assert_eq!(content.len(), 1);
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn test_user_message_new() {
        let msg = UserMessage::new(vec![ContentBlock::text("test")]);
        assert!(!msg.uuid.is_empty());
        assert_eq!(msg.content.len(), 1);
        assert!(msg.timestamp > 0);
    }

    #[test]
    fn test_assistant_message_new() {
        let msg = AssistantMessage::new(vec![ContentBlock::text("response")]);
        assert!(!msg.uuid.is_empty());
        assert_eq!(msg.content.len(), 1);
        assert!(msg.model.is_none());
        assert!(msg.stop_reason.is_none());
    }

    #[test]
    fn test_message_timestamp() {
        let user = Message::User(UserMessage::new(vec![ContentBlock::text("hi")]));
        assert!(user.timestamp().is_some());

        let system = Message::System(SystemMessage {
            content: "sys".to_string(),
        });
        assert!(system.timestamp().is_none());
    }

    #[test]
    fn test_message_is_tool_result() {
        let tool_result = Message::User(UserMessage {
            uuid: "test".to_string(),
            content: vec![ContentBlock::tool_result(
                "id1".to_string(),
                vec![ContentBlock::text("out")],
                false,
            )],
            timestamp: 0,
        });
        assert!(tool_result.is_tool_result());

        let text_msg = Message::User(UserMessage {
            uuid: "test".to_string(),
            content: vec![ContentBlock::text("hello")],
            timestamp: 0,
        });
        assert!(!text_msg.is_tool_result());

        let assistant = Message::Assistant(AssistantMessage::new(vec![ContentBlock::text("hi")]));
        assert!(!assistant.is_tool_result());
    }

    #[test]
    fn test_messages_to_api_params_filters_system() {
        let messages = vec![
            Message::System(SystemMessage {
                content: "sys".to_string(),
            }),
            Message::User(UserMessage {
                uuid: "u1".to_string(),
                content: vec![ContentBlock::text("hello")],
                timestamp: 0,
            }),
            Message::Assistant(AssistantMessage {
                uuid: "a1".to_string(),
                content: vec![ContentBlock::text("world")],
                timestamp: 0,
                model: None,
                stop_reason: None,
                usage: None,
            }),
        ];
        let params = messages_to_api_params(&messages);
        assert_eq!(params.len(), 2);
        assert_eq!(params[0]["role"], "user");
        assert_eq!(params[1]["role"], "assistant");
    }

    #[test]
    fn test_content_blocks_to_api_text() {
        let blocks = vec![ContentBlock::text("hello")];
        let api = content_blocks_to_api(&blocks);
        assert_eq!(api[0]["type"], "text");
        assert_eq!(api[0]["text"], "hello");
    }

    #[test]
    fn test_content_blocks_to_api_tool_use() {
        let blocks = vec![ContentBlock::ToolUse {
            id: "tu_1".to_string(),
            name: "Bash".to_string(),
            input: serde_json::json!({"command": "ls"}),
        }];
        let api = content_blocks_to_api(&blocks);
        assert_eq!(api[0]["type"], "tool_use");
        assert_eq!(api[0]["name"], "Bash");
    }

    #[test]
    fn test_content_blocks_to_api_thinking() {
        let blocks = vec![ContentBlock::Thinking {
            thinking: "reasoning".to_string(),
            signature: "sig".to_string(),
        }];
        let api = content_blocks_to_api(&blocks);
        assert_eq!(api[0]["type"], "thinking");
        assert_eq!(api[0]["thinking"], "reasoning");
    }

    #[test]
    fn test_message_serialization_roundtrip() {
        let msg = Message::User(UserMessage {
            uuid: "test-uuid".to_string(),
            content: vec![ContentBlock::text("hello world")],
            timestamp: 1234567890,
        });
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: Message = serde_json::from_str(&json).unwrap();
        match deserialized {
            Message::User(u) => {
                assert_eq!(u.uuid, "test-uuid");
                assert_eq!(u.timestamp, 1234567890);
            }
            _ => panic!("expected User message"),
        }
    }
}
