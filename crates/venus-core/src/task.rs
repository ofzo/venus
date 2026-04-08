use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub active_form: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Deleted,
}

/// Updates to apply to a task.
pub struct TaskUpdate {
    pub status: Option<TaskStatus>,
    pub subject: Option<String>,
    pub description: Option<String>,
    pub active_form: Option<String>,
    pub owner: Option<String>,
    pub add_blocks: Option<Vec<String>>,
    pub add_blocked_by: Option<Vec<String>>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskStore {
    tasks: Arc<RwLock<Vec<Task>>>,
    next_id: Arc<RwLock<u64>>,
}

impl TaskStore {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(Vec::new())),
            next_id: Arc::new(RwLock::new(1)),
        }
    }

    pub fn create(
        &self,
        subject: String,
        description: String,
        active_form: Option<String>,
    ) -> Task {
        let mut next_id = self.next_id.write().expect("next_id lock poisoned");
        let id = format!("task_{}", *next_id);
        *next_id += 1;

        let task = Task {
            id,
            subject,
            description,
            status: TaskStatus::Pending,
            active_form,
            owner: None,
            blocked_by: Vec::new(),
            blocks: Vec::new(),
            metadata: HashMap::new(),
        };

        let mut tasks = self.tasks.write().expect("tasks lock poisoned");
        tasks.push(task.clone());
        task
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        let tasks = self.tasks.read().expect("tasks lock poisoned");
        tasks.iter().find(|t| t.id == id).cloned()
    }

    /// List all non-deleted tasks.
    pub fn list(&self) -> Vec<Task> {
        let tasks = self.tasks.read().expect("tasks lock poisoned");
        tasks
            .iter()
            .filter(|t| t.status != TaskStatus::Deleted)
            .cloned()
            .collect()
    }

    pub fn update(&self, id: &str, updates: TaskUpdate) -> Option<Task> {
        let mut tasks = self.tasks.write().expect("tasks lock poisoned");
        let task = tasks.iter_mut().find(|t| t.id == id)?;

        if let Some(status) = updates.status {
            task.status = status;
        }
        if let Some(subject) = updates.subject {
            task.subject = subject;
        }
        if let Some(description) = updates.description {
            task.description = description;
        }
        if let Some(active_form) = updates.active_form {
            task.active_form = Some(active_form);
        }
        if let Some(owner) = updates.owner {
            task.owner = Some(owner);
        }
        if let Some(add_blocks) = updates.add_blocks {
            for b in add_blocks {
                if !task.blocks.contains(&b) {
                    task.blocks.push(b);
                }
            }
        }
        if let Some(add_blocked_by) = updates.add_blocked_by {
            for b in add_blocked_by {
                if !task.blocked_by.contains(&b) {
                    task.blocked_by.push(b);
                }
            }
        }
        if let Some(metadata) = updates.metadata {
            task.metadata.extend(metadata);
        }

        Some(task.clone())
    }
}
