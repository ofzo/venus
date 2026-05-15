use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct MonitorTool;

#[async_trait]
impl Tool for MonitorTool {
    fn name(&self) -> &str {
        "Monitor"
    }

    fn description(&self) -> &str {
        "Watch a file or directory for changes using polling. Returns the current status and any detected changes."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file or directory to watch"
                },
                "pattern": {
                    "type": "string",
                    "description": "Optional glob pattern to filter files (e.g., '*.rs')"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'path' parameter"))?;

        let path = if Path::new(path_str).is_absolute() {
            PathBuf::from(path_str)
        } else {
            ctx.working_dir.join(path_str)
        };

        if !path.exists() {
            return Ok(ToolResult::error(format!(
                "path does not exist: {}",
                path.display()
            )));
        }

        let pattern = input.get("pattern").and_then(|v| v.as_str());

        // Take an initial snapshot
        let initial = snapshot_metadata(&path)?;

        // Poll briefly (1 second) to detect changes
        tokio::time::sleep(Duration::from_millis(1000)).await;

        let current = snapshot_metadata(&path)?;

        let changes = if initial != current {
            "Changes detected".to_string()
        } else {
            "No changes detected".to_string()
        };

        let mut info = format!(
            "Monitoring: {}\nPattern: {}\n{}\nCurrent state: {} items, last modified: {}",
            path.display(),
            pattern.unwrap_or("(none)"),
            changes,
            current.item_count,
            current.last_modified,
        );

        if let Some(pat) = pattern {
            if path.is_dir() {
                let matching = count_matching_files(&path, pat);
                info.push_str(&format!("\nMatching files (pattern '{}'): {}", pat, matching));
            }
        }

        Ok(ToolResult::text(info))
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn format_for_display(&self, input: &Value) -> String {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
        format!("Monitor: {}", path)
    }
}

#[derive(Debug, PartialEq)]
struct PathSnapshot {
    item_count: u64,
    last_modified: u64,
    size: u64,
}

fn snapshot_metadata(path: &Path) -> Result<PathSnapshot> {
    if path.is_file() {
        let meta = std::fs::metadata(path)?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Ok(PathSnapshot {
            item_count: 1,
            last_modified: modified,
            size: meta.len(),
        })
    } else if path.is_dir() {
        let mut count = 0u64;
        let mut latest_mod = 0u64;
        let mut total_size = 0u64;

        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                count += 1;
                if let Ok(meta) = entry.metadata() {
                    total_size += meta.len();
                    if let Ok(modified) = meta.modified() {
                        if let Ok(d) = modified.duration_since(std::time::UNIX_EPOCH) {
                            let secs = d.as_secs();
                            if secs > latest_mod {
                                latest_mod = secs;
                            }
                        }
                    }
                }
            }
        }

        Ok(PathSnapshot {
            item_count: count,
            last_modified: latest_mod,
            size: total_size,
        })
    } else {
        Err(anyhow::anyhow!("path is neither a file nor a directory"))
    }
}

fn count_matching_files(dir: &Path, pattern: &str) -> u64 {
    let glob = match globset::Glob::new(pattern) {
        Ok(g) => g.compile_matcher(),
        Err(_) => return 0,
    };

    let mut count = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if glob.is_match(&name) {
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;
    use venus_core::background::BackgroundTaskRuntime;
    use venus_core::hooks::HookRunner;
    use venus_core::task::TaskStore;
    use venus_core::tool::{PermissionDecision, PermissionHandler};
    use venus_core::tool_registry::ToolRegistry;
    use venus_utils::config::Settings;

    struct NoopPermission;
    #[async_trait]
    impl PermissionHandler for NoopPermission {
        async fn check_permission(&self, _: &str, _: &Value) -> PermissionDecision {
            PermissionDecision::Allow
        }
    }

    fn make_context(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            working_dir: dir.to_path_buf(),
            session_id: "test-session".to_string(),
            cancel_token: CancellationToken::new(),
            permission_handler: Arc::new(NoopPermission),
            settings: Arc::new(Settings::default()),
            task_store: Arc::new(TaskStore::new()),
            background_runtime: Arc::new(BackgroundTaskRuntime::new()),
            plan_mode: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            auth_header: "",
            auth_value: String::new(),
            base_url: String::new(),
            model: String::new(),
            tools: Arc::new(ToolRegistry::new(vec![])),
            hook_runner: Arc::new(HookRunner::new(None, "test-session".to_string(), dir.to_path_buf())),
            cron_scheduler: None,
        }
    }

    #[tokio::test]
    async fn test_monitor_nonexistent_path() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_context(tmp.path());
        let tool = MonitorTool;
        let input = serde_json::json!({ "path": "/nonexistent/path/abc123" });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_monitor_existing_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("test.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let ctx = make_context(tmp.path());
        let tool = MonitorTool;
        let input = serde_json::json!({ "path": "test.txt" });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content[0].as_text().unwrap().contains("Monitoring"));
    }

    #[test]
    fn test_snapshot_metadata_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let snap = snapshot_metadata(&file_path).unwrap();
        assert_eq!(snap.item_count, 1);
        assert_eq!(snap.size, 11);
    }
}
