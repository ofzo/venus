use anyhow::Result;
use async_trait::async_trait;
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use venus_core::tool::{Tool, ToolContext, ToolResult};

pub struct MonitorTool;

#[async_trait]
impl Tool for MonitorTool {
    fn name(&self) -> &str {
        "Monitor"
    }

    fn description(&self) -> &str {
        "Watch a file or directory for changes using file system events. \
         Returns detected changes within the specified duration."
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
                },
                "duration_secs": {
                    "type": "number",
                    "description": "How long to watch in seconds (default: 5, max: 30)"
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
        let duration_secs = input
            .get("duration_secs")
            .and_then(|v| v.as_f64())
            .unwrap_or(5.0)
            .clamp(1.0, 30.0);

        let glob_matcher = pattern.and_then(|p| globset::Glob::new(p).ok().map(|g| g.compile_matcher()));

        // Set up file system watcher
        let (tx, rx) = std_mpsc::channel();
        let mut watcher = RecommendedWatcher::new(
            tx,
            Config::default().with_poll_interval(Duration::from_millis(200)),
        )?;

        let watch_path = path.clone();
        watcher.watch(&watch_path, RecursiveMode::Recursive)?;

        // Collect events for the specified duration
        let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(duration_secs);
        let mut events: Vec<String> = Vec::new();

        while tokio::time::Instant::now() < deadline {
            match rx.try_recv() {
                Ok(Ok(event)) => {
                    let kind_str = match event.kind {
                        EventKind::Create(_) => "created",
                        EventKind::Modify(_) => "modified",
                        EventKind::Remove(_) => "removed",
                        EventKind::Access(_) => "accessed",
                        _ => "other",
                    };

                    for event_path in &event.paths {
                        // Apply pattern filter if specified
                        if let Some(ref matcher) = glob_matcher {
                            let file_name = event_path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("");
                            if !matcher.is_match(file_name) {
                                continue;
                            }
                        }

                        let rel_path = event_path
                            .strip_prefix(&path)
                            .unwrap_or(event_path)
                            .display();
                        events.push(format!("{}: {}", kind_str, rel_path));
                    }
                }
                Ok(Err(e)) => {
                    events.push(format!("watch error: {}", e));
                }
                Err(std_mpsc::TryRecvError::Empty) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(std_mpsc::TryRecvError::Disconnected) => break,
            }
        }

        // Stop watching
        drop(watcher);

        // Build result
        let initial = snapshot_metadata(&path)?;
        let mut info = format!(
            "Monitoring: {}\nDuration: {:.1}s\nPattern: {}\nCurrent state: {} items, {} bytes",
            path.display(),
            duration_secs,
            pattern.unwrap_or("(none)"),
            initial.item_count,
            initial.size,
        );

        if events.is_empty() {
            info.push_str("\n\nNo changes detected during monitoring period.");
        } else {
            // Deduplicate events
            events.sort();
            events.dedup();
            let show_count = events.len().min(50);
            info.push_str(&format!("\n\nChanges detected ({}):\n", events.len()));
            for event in events.iter().take(show_count) {
                info.push_str(&format!("  {}\n", event));
            }
            if events.len() > show_count {
                info.push_str(&format!("  ... and {} more\n", events.len() - show_count));
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
    size: u64,
}

fn snapshot_metadata(path: &Path) -> Result<PathSnapshot> {
    if path.is_file() {
        let meta = std::fs::metadata(path)?;
        Ok(PathSnapshot {
            item_count: 1,
            size: meta.len(),
        })
    } else if path.is_dir() {
        let mut count = 0u64;
        let mut total_size = 0u64;

        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                count += 1;
                if let Ok(meta) = entry.metadata() {
                    total_size += meta.len();
                }
            }
        }

        Ok(PathSnapshot {
            item_count: count,
            size: total_size,
        })
    } else {
        Err(anyhow::anyhow!("path is neither a file nor a directory"))
    }
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
            messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            auth_header: "",
            auth_value: String::new(),
            base_url: String::new(),
            model: String::new(),
            tools: Arc::new(ToolRegistry::new(vec![])),
            hook_runner: Arc::new(HookRunner::new(None, "test-session".to_string(), dir.to_path_buf())),
            cron_scheduler: None,
            cost_tracker: None,
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
        let input = serde_json::json!({ "path": "test.txt", "duration_secs": 1.0 });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Monitoring"));
        assert!(text.contains("No changes detected"));
    }

    #[tokio::test]
    async fn test_monitor_detects_change() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("watched.txt");
        std::fs::write(&file_path, "initial").unwrap();

        let path_clone = file_path.clone();
        let ctx = make_context(tmp.path());
        let tool = MonitorTool;
        let input = serde_json::json!({ "path": "watched.txt", "duration_secs": 2.0 });

        // Spawn a task to modify the file after a short delay
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            std::fs::write(&path_clone, "modified").unwrap();
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("Monitoring"));
        // Should detect the change
        assert!(text.contains("modified") || text.contains("Changes detected"));
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

    #[test]
    fn test_snapshot_metadata_dir() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "aaa").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "bb").unwrap();

        let snap = snapshot_metadata(tmp.path()).unwrap();
        assert_eq!(snap.item_count, 2);
        assert_eq!(snap.size, 5);
    }
}
