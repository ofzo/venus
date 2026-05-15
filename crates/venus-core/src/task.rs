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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_task() {
        let store = TaskStore::new();
        let task = store.create("Test".to_string(), "Description".to_string(), None);
        assert_eq!(task.id, "task_1");
        assert_eq!(task.subject, "Test");
        assert_eq!(task.description, "Description");
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.active_form.is_none());
        assert!(task.owner.is_none());
        assert!(task.blocked_by.is_empty());
        assert!(task.blocks.is_empty());
    }

    #[test]
    fn test_create_multiple_tasks_incrementing_ids() {
        let store = TaskStore::new();
        let t1 = store.create("A".to_string(), "".to_string(), None);
        let t2 = store.create("B".to_string(), "".to_string(), None);
        assert_eq!(t1.id, "task_1");
        assert_eq!(t2.id, "task_2");
    }

    #[test]
    fn test_get_existing_task() {
        let store = TaskStore::new();
        store.create("Find me".to_string(), "".to_string(), None);
        let found = store.get("task_1");
        assert!(found.is_some());
        assert_eq!(found.unwrap().subject, "Find me");
    }

    #[test]
    fn test_get_nonexistent_task() {
        let store = TaskStore::new();
        assert!(store.get("task_999").is_none());
    }

    #[test]
    fn test_list_excludes_deleted() {
        let store = TaskStore::new();
        store.create("A".to_string(), "".to_string(), None);
        store.create("B".to_string(), "".to_string(), None);
        store.create("C".to_string(), "".to_string(), None);

        store.update("task_2", TaskUpdate {
            status: Some(TaskStatus::Deleted),
            subject: None,
            description: None,
            active_form: None,
            owner: None,
            add_blocks: None,
            add_blocked_by: None,
            metadata: None,
        });

        let tasks = store.list();
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().all(|t| t.id != "task_2"));
    }

    #[test]
    fn test_update_status() {
        let store = TaskStore::new();
        store.create("Task".to_string(), "".to_string(), None);

        let updated = store.update("task_1", TaskUpdate {
            status: Some(TaskStatus::InProgress),
            subject: None,
            description: None,
            active_form: None,
            owner: None,
            add_blocks: None,
            add_blocked_by: None,
            metadata: None,
        }).unwrap();

        assert_eq!(updated.status, TaskStatus::InProgress);
    }

    #[test]
    fn test_update_subject_and_description() {
        let store = TaskStore::new();
        store.create("Old".to_string(), "Old desc".to_string(), None);

        let updated = store.update("task_1", TaskUpdate {
            status: None,
            subject: Some("New".to_string()),
            description: Some("New desc".to_string()),
            active_form: None,
            owner: None,
            add_blocks: None,
            add_blocked_by: None,
            metadata: None,
        }).unwrap();

        assert_eq!(updated.subject, "New");
        assert_eq!(updated.description, "New desc");
    }

    #[test]
    fn test_update_add_blocks() {
        let store = TaskStore::new();
        store.create("Task".to_string(), "".to_string(), None);

        let updated = store.update("task_1", TaskUpdate {
            status: None,
            subject: None,
            description: None,
            active_form: None,
            owner: None,
            add_blocks: Some(vec!["task_2".to_string(), "task_3".to_string()]),
            add_blocked_by: None,
            metadata: None,
        }).unwrap();

        assert_eq!(updated.blocks, vec!["task_2", "task_3"]);

        // Adding duplicate should not duplicate
        let updated2 = store.update("task_1", TaskUpdate {
            status: None,
            subject: None,
            description: None,
            active_form: None,
            owner: None,
            add_blocks: Some(vec!["task_2".to_string()]),
            add_blocked_by: None,
            metadata: None,
        }).unwrap();

        assert_eq!(updated2.blocks.len(), 2);
    }

    #[test]
    fn test_update_add_blocked_by() {
        let store = TaskStore::new();
        store.create("Task".to_string(), "".to_string(), None);

        let updated = store.update("task_1", TaskUpdate {
            status: None,
            subject: None,
            description: None,
            active_form: None,
            owner: None,
            add_blocks: None,
            add_blocked_by: Some(vec!["task_0".to_string()]),
            metadata: None,
        }).unwrap();

        assert_eq!(updated.blocked_by, vec!["task_0"]);
    }

    #[test]
    fn test_update_metadata() {
        let store = TaskStore::new();
        store.create("Task".to_string(), "".to_string(), None);

        let mut meta = HashMap::new();
        meta.insert("key".to_string(), serde_json::json!("value"));

        let updated = store.update("task_1", TaskUpdate {
            status: None,
            subject: None,
            description: None,
            active_form: None,
            owner: None,
            add_blocks: None,
            add_blocked_by: None,
            metadata: Some(meta),
        }).unwrap();

        assert_eq!(updated.metadata["key"], serde_json::json!("value"));
    }

    #[test]
    fn test_update_nonexistent_returns_none() {
        let store = TaskStore::new();
        let result = store.update("task_999", TaskUpdate {
            status: Some(TaskStatus::Completed),
            subject: None,
            description: None,
            active_form: None,
            owner: None,
            add_blocks: None,
            add_blocked_by: None,
            metadata: None,
        });
        assert!(result.is_none());
    }

    #[test]
    fn test_task_status_serialization() {
        assert_eq!(serde_json::to_string(&TaskStatus::Pending).unwrap(), "\"pending\"");
        assert_eq!(serde_json::to_string(&TaskStatus::InProgress).unwrap(), "\"in_progress\"");
        assert_eq!(serde_json::to_string(&TaskStatus::Completed).unwrap(), "\"completed\"");
        assert_eq!(serde_json::to_string(&TaskStatus::Deleted).unwrap(), "\"deleted\"");
    }

    #[test]
    fn test_task_serialization_roundtrip() {
        let task = Task {
            id: "task_1".to_string(),
            subject: "Test".to_string(),
            description: "Desc".to_string(),
            status: TaskStatus::Pending,
            active_form: Some("Working".to_string()),
            owner: None,
            blocked_by: vec!["task_0".to_string()],
            blocks: vec![],
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&task).unwrap();
        let deserialized: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "task_1");
        assert_eq!(deserialized.status, TaskStatus::Pending);
        assert_eq!(deserialized.blocked_by, vec!["task_0"]);
    }
}
