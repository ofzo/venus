use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BackgroundTaskStatus {
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundTaskInfo {
    pub id: String,
    pub description: String,
    pub started_at: u64,
    pub status: BackgroundTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output: Option<String>,
}

struct BackgroundTaskEntry {
    info: BackgroundTaskInfo,
    cancel_token: CancellationToken,
    output: Arc<Mutex<String>>,
}

#[derive(Clone)]
pub struct BackgroundTaskRuntime {
    tasks: Arc<RwLock<HashMap<String, BackgroundTaskEntry>>>,
    next_id: Arc<AtomicU64>,
}

impl BackgroundTaskRuntime {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Spawn a background task. Returns the task ID immediately.
    pub async fn spawn<F>(&self, description: String, task_fn: F) -> String
    where
        F: std::future::Future<Output = Result<String, String>> + Send + 'static,
    {
        let id = format!("bg_{}", self.next_id.fetch_add(1, Ordering::SeqCst));
        let cancel_token = CancellationToken::new();
        let output = Arc::new(Mutex::new(String::new()));
        let now = chrono::Utc::now().timestamp() as u64;

        let entry = BackgroundTaskEntry {
            info: BackgroundTaskInfo {
                id: id.clone(),
                description,
                started_at: now,
                status: BackgroundTaskStatus::Running,
                output: None,
            },
            cancel_token: cancel_token.clone(),
            output: output.clone(),
        };

        self.tasks.write().await.insert(id.clone(), entry);

        let tasks = self.tasks.clone();
        let tasks_for_save = self.tasks.clone();
        let task_id = id.clone();
        let token = cancel_token.clone();

        tokio::spawn(async move {
            let result = tokio::select! {
                result = task_fn => result,
                _ = token.cancelled() => Err("cancelled".to_string()),
            };

            {
                let mut tasks_guard = tasks.write().await;
                if let Some(entry) = tasks_guard.get_mut(&task_id) {
                    match result {
                        Ok(out) => {
                            entry.info.output = Some(out.clone());
                            *entry.output.lock().await = out;
                            entry.info.status = BackgroundTaskStatus::Completed;
                        }
                        Err(e) => {
                            if e == "cancelled" {
                                entry.info.status = BackgroundTaskStatus::Cancelled;
                            } else {
                                entry.info.output = Some(e.clone());
                                *entry.output.lock().await = e.clone();
                                entry.info.status = BackgroundTaskStatus::Failed(e);
                            }
                        }
                    }
                }
            }

            // Auto-save to disk for cross-session persistence
            let _ = Self::save_tasks_to_disk_static(&tasks_for_save).await;
        });

        id
    }

    /// Read output and status of a background task.
    pub async fn read_output(&self, task_id: &str) -> Result<(BackgroundTaskInfo, String)> {
        let tasks = self.tasks.read().await;
        let entry = tasks
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("background task '{}' not found", task_id))?;
        let output = entry.output.lock().await.clone();
        Ok((entry.info.clone(), output))
    }

    /// Cancel a running background task.
    pub async fn cancel(&self, task_id: &str) -> Result<bool> {
        let tasks = self.tasks.read().await;
        if let Some(entry) = tasks.get(task_id) {
            if entry.info.status == BackgroundTaskStatus::Running {
                entry.cancel_token.cancel();
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// List all background tasks.
    pub async fn list(&self) -> Vec<BackgroundTaskInfo> {
        self.tasks
            .read()
            .await
            .values()
            .map(|e| e.info.clone())
            .collect()
    }

    /// Get the persistence file path: ~/.venus/background_tasks.json
    fn persistence_path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".venus").join("background_tasks.json")
    }

    /// Save current task list to disk.
    pub async fn save_to_disk(&self) -> Result<()> {
        let tasks = self.list().await;
        Self::save_tasks_to_disk(&tasks).await
    }

    /// Save a task list to disk (static helper).
    async fn save_tasks_to_disk(tasks: &[BackgroundTaskInfo]) -> Result<()> {
        let path = Self::persistence_path();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_string_pretty(tasks)?;
        tokio::fs::write(&path, json).await?;
        Ok(())
    }

    /// Save tasks from the internal HashMap to disk (used in spawned task).
    async fn save_tasks_to_disk_static(tasks: &RwLock<HashMap<String, BackgroundTaskEntry>>) -> Result<()> {
        let tasks = tasks.read().await;
        let infos: Vec<BackgroundTaskInfo> = tasks.values().map(|e| e.info.clone()).collect();
        Self::save_tasks_to_disk(&infos).await
    }

    /// Load task list from disk (for display only, not restoring running tasks).
    pub async fn load_from_disk() -> Result<Vec<BackgroundTaskInfo>> {
        let path = Self::persistence_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let data = tokio::fs::read_to_string(&path).await?;
        let tasks: Vec<BackgroundTaskInfo> = serde_json::from_str(&data)?;
        Ok(tasks)
    }
}

impl Default for BackgroundTaskRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_and_complete() {
        let runtime = BackgroundTaskRuntime::new();
        let id = runtime
            .spawn("test task".to_string(), async { Ok("output".to_string()) })
            .await;

        assert!(id.starts_with("bg_"));

        // Wait a bit for the task to complete
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let (info, output) = runtime.read_output(&id).await.unwrap();
        assert_eq!(info.status, BackgroundTaskStatus::Completed);
        assert_eq!(output, "output");
    }

    #[tokio::test]
    async fn test_spawn_and_fail() {
        let runtime = BackgroundTaskRuntime::new();
        let id = runtime
            .spawn("failing task".to_string(), async {
                Err("something went wrong".to_string())
            })
            .await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let (info, output) = runtime.read_output(&id).await.unwrap();
        assert_eq!(
            info.status,
            BackgroundTaskStatus::Failed("something went wrong".to_string())
        );
        assert_eq!(output, "something went wrong");
    }

    #[tokio::test]
    async fn test_cancel_running_task() {
        let runtime = BackgroundTaskRuntime::new();
        let id = runtime
            .spawn("long task".to_string(), async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok("done".to_string())
            })
            .await;

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let cancelled = runtime.cancel(&id).await.unwrap();
        assert!(cancelled);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let (info, _) = runtime.read_output(&id).await.unwrap();
        assert_eq!(info.status, BackgroundTaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_cancel_nonexistent() {
        let runtime = BackgroundTaskRuntime::new();
        let result = runtime.cancel("bg_999").await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_read_output_nonexistent() {
        let runtime = BackgroundTaskRuntime::new();
        let result = runtime.read_output("bg_999").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_tasks() {
        let runtime = BackgroundTaskRuntime::new();
        runtime
            .spawn("task 1".to_string(), async { Ok("a".to_string()) })
            .await;
        runtime
            .spawn("task 2".to_string(), async { Ok("b".to_string()) })
            .await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let tasks = runtime.list().await;
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().all(|t| t.status == BackgroundTaskStatus::Completed));
    }

    #[tokio::test]
    async fn test_task_ids_are_sequential() {
        let runtime = BackgroundTaskRuntime::new();
        let id1 = runtime
            .spawn("a".to_string(), async { Ok("".to_string()) })
            .await;
        let id2 = runtime
            .spawn("b".to_string(), async { Ok("".to_string()) })
            .await;
        assert_eq!(id1, "bg_1");
        assert_eq!(id2, "bg_2");
    }
}
