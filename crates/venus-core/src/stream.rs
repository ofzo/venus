use crate::message::AssistantMessage;
use venus_utils::cost::TokenUsage;

/// Events emitted during a streaming query.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Incremental text from the assistant.
    TextDelta(String),
    /// Incremental thinking text.
    ThinkingDelta(String),
    /// A tool use block has started.
    ToolUseStart { id: String, name: String },
    /// Incremental JSON input for a tool.
    ToolUseInput(String),
    /// A tool has completed execution.
    ToolResult {
        id: String,
        name: String,
        result: crate::tool::ToolResult,
    },
    /// The complete assistant message.
    MessageComplete(AssistantMessage),
    /// An error occurred.
    Error(String),
    /// Token usage update.
    Usage(TokenUsage),
}
