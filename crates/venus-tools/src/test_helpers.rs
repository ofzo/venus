use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use venus_core::background::BackgroundTaskRuntime;
use venus_core::task::TaskStore;
use venus_core::tool::{PermissionDecision, PermissionHandler, ToolContext};
use venus_utils::config::Settings;

struct NoopPerm;

#[async_trait]
impl PermissionHandler for NoopPerm {
    async fn check_permission(&self, _tool_name: &str, _input: &Value) -> PermissionDecision {
        PermissionDecision::Allow
    }
}

pub fn make_test_context() -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("/tmp"),
        session_id: "test".to_string(),
        cancel_token: tokio_util::sync::CancellationToken::new(),
        permission_handler: Arc::new(NoopPerm),
        settings: Arc::new(Settings::default()),
        task_store: Arc::new(TaskStore::new()),
        background_runtime: Arc::new(BackgroundTaskRuntime::new()),
        plan_mode: Arc::new(AtomicBool::new(false)),
        auth_header: "",
        auth_value: String::new(),
        base_url: String::new(),
        model: String::new(),
        tools: Arc::new(venus_core::tool_registry::ToolRegistry::new(vec![])),
        hook_runner: Arc::new(venus_core::hooks::HookRunner::new(
            None,
            String::new(),
            PathBuf::from("/tmp"),
        )),
        cron_scheduler: None,
    }
}
