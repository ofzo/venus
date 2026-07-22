use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::background::BackgroundTaskRuntime;
use crate::message::{ContentBlock, Message};
use crate::task::TaskStore;
use tokio::sync::Mutex;
use venus_utils::config::Settings;

/// Result of a tool execution.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
    /// Optional structured diff captured by file-write/edit tools. When
    /// present, the TUI renders a colourised `+/-` diff block under the tool
    /// header instead of (or alongside) the plain status-line body. `None` for
    /// all non-file tools and for no-op / error tool calls.
    pub diff: Option<venus_utils::diff::ToolDiff>,
}

impl ToolResult {
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::text(s)],
            is_error: false,
            diff: None,
        }
    }

    pub fn error(s: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::text(s)],
            is_error: true,
            diff: None,
        }
    }

    /// Attach the previously-computed structured file diff. Consumes `self`
    /// so the caller cannot forget to return the built result.
    pub fn with_diff(mut self, diff: venus_utils::diff::ToolDiff) -> Self {
        self.diff = Some(diff);
        self
    }
}

/// Permission handler trait - implemented by the permission system.
#[async_trait]
pub trait PermissionHandler: Send + Sync {
    async fn check_permission(&self, tool_name: &str, input: &Value) -> PermissionDecision;
}

#[derive(Debug, Clone)]
pub enum PermissionDecision {
    Allow,
    Deny(String),
    Ask(String), // description of what the tool wants to do
}

/// Context passed to tools during execution.
pub struct ToolContext {
    pub working_dir: PathBuf,
    pub session_id: String,
    pub cancel_token: CancellationToken,
    pub permission_handler: Arc<dyn PermissionHandler>,
    pub settings: Arc<Settings>,
    pub task_store: Arc<TaskStore>,
    pub background_runtime: Arc<BackgroundTaskRuntime>,
    pub plan_mode: Arc<AtomicBool>,
    /// Shared message history - tools can read and modify conversation.
    pub messages: Arc<Mutex<Vec<Message>>>,
    // Fields needed for sub-agent spawning
    pub auth_header: &'static str,
    pub auth_value: String,
    pub base_url: String,
    pub model: String,
    pub tools: Arc<crate::tool_registry::ToolRegistry>,
    pub hook_runner: Arc<crate::hooks::HookRunner>,
    pub cron_scheduler: Option<Arc<crate::cron::CronScheduler>>,
    /// Parent engine's cost tracker, shared by sub-agents spawned via
    /// the `Agent` tool. When `Some`, the sub-agent records its token
    /// usage into the *same* tracker so the parent's `cost_tracker`
    /// reflects the true total (main + sub-agent) consumption; tests
    /// can leave this `None` to fall back to an isolated tracker.
    pub cost_tracker: Option<Arc<std::sync::Mutex<venus_utils::cost::CostTracker>>>,
}

/// The core Tool trait that all tools implement.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique name of the tool.
    fn name(&self) -> &str;

    /// Human-readable description of the tool.
    fn description(&self) -> &str;

    /// JSON Schema for the tool's input.
    fn input_schema(&self) -> Value;

    /// Execute the tool with the given input.
    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult>;

    /// Whether this tool only reads data (no side effects).
    fn is_read_only(&self) -> bool {
        false
    }

    /// Format the tool use for display to the user (for permission prompting).
    fn format_for_display(&self, input: &Value) -> String {
        format!(
            "{}: {}",
            self.name(),
            serde_json::to_string_pretty(input).unwrap_or_default()
        )
    }

    /// Convert to the API tool definition format.
    fn to_api_definition(&self) -> Value {
        serde_json::json!({
            "name": self.name(),
            "description": self.description(),
            "input_schema": self.input_schema(),
        })
    }
}
