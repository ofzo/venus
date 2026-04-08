use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundTaskStatus {
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct BackgroundTaskInfo {
    pub id: String,
    pub description: String,
    pub started_at: u64,
    pub status: BackgroundTaskStatus,
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
            },
            cancel_token: cancel_token.clone(),
            output: output.clone(),
        };

        self.tasks.write().await.insert(id.clone(), entry);

        let tasks = self.tasks.clone();
        let task_id = id.clone();
        let token = cancel_token.clone();

        tokio::spawn(async move {
            let result = tokio::select! {
                result = task_fn => result,
                _ = token.cancelled() => Err("cancelled".to_string()),
            };

            let mut tasks = tasks.write().await;
            if let Some(entry) = tasks.get_mut(&task_id) {
                match result {
                    Ok(out) => {
                        *entry.output.lock().await = out;
                        entry.info.status = BackgroundTaskStatus::Completed;
                    }
                    Err(e) => {
                        if e == "cancelled" {
                            entry.info.status = BackgroundTaskStatus::Cancelled;
                        } else {
                            *entry.output.lock().await = e.clone();
                            entry.info.status = BackgroundTaskStatus::Failed(e);
                        }
                    }
                }
            }
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
}
