use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::fs_helpers::claude_config_dir;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Feedback => write!(f, "feedback"),
            Self::Project => write!(f, "project"),
            Self::Reference => write!(f, "reference"),
        }
    }
}

impl std::str::FromStr for MemoryType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "user" => Ok(Self::User),
            "feedback" => Ok(Self::Feedback),
            "project" => Ok(Self::Project),
            "reference" => Ok(Self::Reference),
            _ => anyhow::bail!("unknown memory type: {}", s),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub id: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub content: String,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Get the user-level memory directory: `~/.claude/memory/`
fn user_memory_dir() -> Result<PathBuf> {
    Ok(claude_config_dir()?.join("memory"))
}

/// Get memory directory. `project_root=None` -> user-level, `Some` -> project-level.
pub fn memory_dir(project_root: Option<&Path>) -> Result<PathBuf> {
    match project_root {
        Some(root) => Ok(root.join(".claude").join("memory")),
        None => user_memory_dir(),
    }
}

/// Write a memory entry to disk and rebuild the MEMORY.md index.
pub async fn write_memory(entry: &MemoryEntry, project_root: Option<&Path>) -> Result<()> {
    let dir = memory_dir(project_root)?;
    tokio::fs::create_dir_all(&dir).await.context("failed to create memory dir")?;

    let file_path = dir.join(format!("{}.md", entry.id));
    let content = serialize_entry(entry);
    tokio::fs::write(&file_path, content).await.context("failed to write memory file")?;

    rebuild_index(&dir).await?;
    Ok(())
}

/// Read a single memory by ID. When `project_root` is Some, searches project dir first,
/// then falls back to user dir. When None, searches user dir only.
pub async fn read_memory(id: &str, project_root: Option<&Path>) -> Result<Option<MemoryEntry>> {
    let filename = format!("{}.md", id);

    // Search project dir first if provided
    if let Some(root) = project_root {
        let project_dir = memory_dir(Some(root))?;
        let path = project_dir.join(&filename);
        if path.exists() {
            let content = tokio::fs::read_to_string(&path).await?;
            return Ok(Some(parse_entry(&content)?));
        }
    }

    // Fall back to user dir
    let user_dir = user_memory_dir()?;
    let path = user_dir.join(&filename);
    if path.exists() {
        let content = tokio::fs::read_to_string(&path).await?;
        return Ok(Some(parse_entry(&content)?));
    }

    Ok(None)
}

/// Delete a memory by ID. Returns true if found and deleted.
pub async fn delete_memory(id: &str, project_root: Option<&Path>) -> Result<bool> {
    let filename = format!("{}.md", id);

    // Try project dir first
    if let Some(root) = project_root {
        let project_dir = memory_dir(Some(root))?;
        let path = project_dir.join(&filename);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
            rebuild_index(&project_dir).await?;
            return Ok(true);
        }
    }

    // Try user dir
    let user_dir = user_memory_dir()?;
    let path = user_dir.join(&filename);
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
        rebuild_index(&user_dir).await?;
        return Ok(true);
    }

    Ok(false)
}

/// List all memories, optionally filtered by type.
/// Reads from both user and project dirs (if project_root is provided).
pub async fn list_memories(
    memory_type: Option<MemoryType>,
    project_root: Option<&Path>,
) -> Result<Vec<MemoryEntry>> {
    let mut entries = Vec::new();

    // Collect from user dir
    if let Ok(user_dir) = user_memory_dir() {
        collect_entries_from_dir(&user_dir, &mut entries).await?;
    }

    // Collect from project dir
    if let Some(root) = project_root {
        let project_dir = memory_dir(Some(root))?;
        collect_entries_from_dir(&project_dir, &mut entries).await?;
    }

    // Filter by type if specified
    if let Some(ref mt) = memory_type {
        entries.retain(|e| &e.memory_type == mt);
    }

    // Sort by updated_at descending
    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(entries)
}

/// Load MEMORY.md content for system prompt injection.
/// Merges user-level and project-level MEMORY.md files.
pub async fn load_memory_for_prompt(project_root: Option<&Path>) -> Result<String> {
    let mut parts = Vec::new();

    // Read user-level MEMORY.md
    if let Ok(user_dir) = user_memory_dir() {
        let index_path = user_dir.join("MEMORY.md");
        if index_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&index_path).await {
                if !content.trim().is_empty() {
                    parts.push(format!("## User Memory\n{}", content.trim()));
                }
            }
        }
    }

    // Read project-level MEMORY.md
    if let Some(root) = project_root {
        let project_dir = memory_dir(Some(root))?;
        let index_path = project_dir.join("MEMORY.md");
        if index_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&index_path).await {
                if !content.trim().is_empty() {
                    parts.push(format!("## Project Memory\n{}", content.trim()));
                }
            }
        }
    }

    Ok(parts.join("\n\n"))
}

/// Rebuild MEMORY.md index from all .md files in the given memory directory.
async fn rebuild_index(dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    let mut entries = Vec::new();
    collect_entries_from_dir(dir, &mut entries).await?;

    // Group by memory_type
    let mut groups: HashMap<String, Vec<&MemoryEntry>> = HashMap::new();
    for entry in &entries {
        groups
            .entry(entry.memory_type.to_string())
            .or_default()
            .push(entry);
    }

    let mut index = String::new();
    // Stable ordering of groups
    let mut group_keys: Vec<&String> = groups.keys().collect();
    group_keys.sort();

    for key in group_keys {
        let group = &groups[key];
        index.push_str(&format!("### {}\n\n", capitalize(key)));
        for entry in group {
            index.push_str(&format!("- **{}** (`{}`): {}\n", entry.title, entry.id, first_line(&entry.content)));
        }
        index.push('\n');
    }

    let index_path = dir.join("MEMORY.md");
    tokio::fs::write(&index_path, index.trim_end()).await.context("failed to write MEMORY.md")?;

    Ok(())
}

/// Collect all memory entries from .md files in a directory (excluding MEMORY.md).
async fn collect_entries_from_dir(dir: &Path, entries: &mut Vec<MemoryEntry>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    let mut read_dir = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("MEMORY.md") {
            continue;
        }
        let content = tokio::fs::read_to_string(&path).await?;
        match parse_entry(&content) {
            Ok(mem_entry) => entries.push(mem_entry),
            Err(e) => {
                tracing::warn!("failed to parse memory file {}: {}", path.display(), e);
            }
        }
    }

    Ok(())
}

/// Parse frontmatter from file content. Returns (key-value map, body).
fn parse_frontmatter(content: &str) -> Result<(HashMap<String, String>, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        anyhow::bail!("missing frontmatter delimiter");
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    let after_first = after_first.trim_start_matches(['\r', '\n']);
    let end_pos = after_first
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("missing closing frontmatter delimiter"))?;

    let frontmatter_str = &after_first[..end_pos];
    let body_start = end_pos + 4; // skip "\n---"
    let body = after_first[body_start..].trim_start_matches(['\r', '\n']).to_string();

    let mut map = HashMap::new();
    for line in frontmatter_str.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }

    Ok((map, body))
}

/// Parse a memory entry from file content.
fn parse_entry(content: &str) -> Result<MemoryEntry> {
    let (map, body) = parse_frontmatter(content)?;

    let id = map.get("id").cloned().unwrap_or_default();
    let memory_type: MemoryType = map
        .get("type")
        .ok_or_else(|| anyhow::anyhow!("missing 'type' in frontmatter"))?
        .parse()?;
    let title = map.get("title").cloned().unwrap_or_default();
    let created_at: u64 = map
        .get("created_at")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let updated_at: u64 = map
        .get("updated_at")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    Ok(MemoryEntry {
        id,
        memory_type,
        title,
        content: body,
        created_at,
        updated_at,
    })
}

/// Serialize a memory entry to frontmatter + body format.
fn serialize_entry(entry: &MemoryEntry) -> String {
    format!(
        "---\nid: {}\ntype: {}\ntitle: {}\ncreated_at: {}\nupdated_at: {}\n---\n{}",
        entry.id, entry.memory_type, entry.title, entry.created_at, entry.updated_at, entry.content
    )
}

/// Get the first line of a string (for index summaries).
fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}

/// Capitalize the first letter of a string.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_entry(id: &str) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            memory_type: MemoryType::User,
            title: "Test Memory".to_string(),
            content: "This is test content.".to_string(),
            created_at: 1712000000,
            updated_at: 1712000000,
        }
    }

    #[test]
    fn test_serialize_and_parse_roundtrip() {
        let entry = sample_entry("abc-123");
        let serialized = serialize_entry(&entry);
        let parsed = parse_entry(&serialized).unwrap();

        assert_eq!(parsed.id, "abc-123");
        assert_eq!(parsed.memory_type, MemoryType::User);
        assert_eq!(parsed.title, "Test Memory");
        assert_eq!(parsed.content, "This is test content.");
        assert_eq!(parsed.created_at, 1712000000);
        assert_eq!(parsed.updated_at, 1712000000);
    }

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = "---\nid: test-1\ntype: project\ntitle: Hello\ncreated_at: 100\nupdated_at: 200\n---\nBody text here";
        let (map, body) = parse_frontmatter(content).unwrap();
        assert_eq!(map.get("id").unwrap(), "test-1");
        assert_eq!(map.get("type").unwrap(), "project");
        assert_eq!(body, "Body text here");
    }

    #[test]
    fn test_parse_frontmatter_missing_delimiter() {
        let content = "no frontmatter here";
        assert!(parse_frontmatter(content).is_err());
    }

    #[test]
    fn test_memory_type_display_and_parse() {
        assert_eq!(MemoryType::User.to_string(), "user");
        assert_eq!(MemoryType::Feedback.to_string(), "feedback");
        assert_eq!("project".parse::<MemoryType>().unwrap(), MemoryType::Project);
        assert_eq!("reference".parse::<MemoryType>().unwrap(), MemoryType::Reference);
        assert!("unknown".parse::<MemoryType>().is_err());
    }

    #[test]
    fn test_memory_dir_user() {
        let dir = memory_dir(None).unwrap();
        assert!(dir.ends_with("memory"));
        assert!(dir.to_string_lossy().contains(".claude"));
    }

    #[test]
    fn test_memory_dir_project() {
        let dir = memory_dir(Some(Path::new("/tmp/myproject"))).unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/myproject/.claude/memory"));
    }

    #[tokio::test]
    async fn test_write_and_read_memory() {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path();
        let entry = sample_entry("write-read-1");

        write_memory(&entry, Some(project_root)).await.unwrap();

        let read_back = read_memory("write-read-1", Some(project_root)).await.unwrap();
        assert!(read_back.is_some());
        let read_back = read_back.unwrap();
        assert_eq!(read_back.id, "write-read-1");
        assert_eq!(read_back.title, "Test Memory");
        assert_eq!(read_back.content, "This is test content.");
    }

    #[tokio::test]
    async fn test_delete_memory() {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path();
        let entry = sample_entry("delete-1");

        write_memory(&entry, Some(project_root)).await.unwrap();
        assert!(read_memory("delete-1", Some(project_root)).await.unwrap().is_some());

        let deleted = delete_memory("delete-1", Some(project_root)).await.unwrap();
        assert!(deleted);

        let after_delete = read_memory("delete-1", Some(project_root)).await.unwrap();
        assert!(after_delete.is_none());
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let deleted = delete_memory("nonexistent", Some(tmp.path())).await.unwrap();
        assert!(!deleted);
    }

    #[tokio::test]
    async fn test_list_memories() {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path();

        let entry1 = MemoryEntry {
            id: "list-1".to_string(),
            memory_type: MemoryType::User,
            title: "First".to_string(),
            content: "Content 1".to_string(),
            created_at: 100,
            updated_at: 200,
        };
        let entry2 = MemoryEntry {
            id: "list-2".to_string(),
            memory_type: MemoryType::Project,
            title: "Second".to_string(),
            content: "Content 2".to_string(),
            created_at: 100,
            updated_at: 300,
        };

        write_memory(&entry1, Some(project_root)).await.unwrap();
        write_memory(&entry2, Some(project_root)).await.unwrap();

        // List all
        let all = list_memories(None, Some(project_root)).await.unwrap();
        assert_eq!(all.len(), 2);
        // Should be sorted by updated_at desc
        assert_eq!(all[0].id, "list-2");

        // Filter by type
        let users = list_memories(Some(MemoryType::User), Some(project_root)).await.unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].id, "list-1");
    }

    #[tokio::test]
    async fn test_rebuild_index() {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path();
        let entry = sample_entry("index-1");

        write_memory(&entry, Some(project_root)).await.unwrap();

        let index_path = memory_dir(Some(project_root)).unwrap().join("MEMORY.md");
        assert!(index_path.exists());

        let index_content = tokio::fs::read_to_string(&index_path).await.unwrap();
        assert!(index_content.contains("Test Memory"));
        assert!(index_content.contains("index-1"));
    }

    #[tokio::test]
    async fn test_load_memory_for_prompt() {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path();
        let entry = sample_entry("prompt-1");

        write_memory(&entry, Some(project_root)).await.unwrap();

        let prompt = load_memory_for_prompt(Some(project_root)).await.unwrap();
        assert!(prompt.contains("Project Memory"));
        assert!(prompt.contains("Test Memory"));
    }

    #[test]
    fn test_first_line() {
        assert_eq!(first_line("hello\nworld"), "hello");
        assert_eq!(first_line("single"), "single");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn test_capitalize() {
        assert_eq!(capitalize("user"), "User");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("a"), "A");
    }

    #[test]
    fn test_multiline_content_roundtrip() {
        let entry = MemoryEntry {
            id: "multi-1".to_string(),
            memory_type: MemoryType::Reference,
            title: "Multi Line".to_string(),
            content: "Line 1\nLine 2\nLine 3".to_string(),
            created_at: 100,
            updated_at: 200,
        };
        let serialized = serialize_entry(&entry);
        let parsed = parse_entry(&serialized).unwrap();
        assert_eq!(parsed.content, "Line 1\nLine 2\nLine 3");
    }
}
