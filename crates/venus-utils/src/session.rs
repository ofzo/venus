use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub project: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: usize,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionFile {
    meta: SessionMeta,
    messages: Vec<serde_json::Value>,
}

/// Get the sessions directory: ~/.venus/sessions/
pub fn sessions_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(".venus").join("sessions"))
}

/// Save messages to a session file: ~/.venus/sessions/{session_id}.json
pub async fn save_session(
    session_id: &str,
    meta: &SessionMeta,
    messages: &[serde_json::Value],
) -> Result<()> {
    let dir = sessions_dir()?;
    tokio::fs::create_dir_all(&dir).await?;

    let file_path = dir.join(format!("{}.json", session_id));
    let session_file = SessionFile {
        meta: meta.clone(),
        messages: messages.to_vec(),
    };

    let json = serde_json::to_string_pretty(&session_file)?;
    tokio::fs::write(&file_path, json).await?;

    Ok(())
}

/// Load a session from disk
pub async fn load_session(
    session_id: &str,
) -> Result<(SessionMeta, Vec<serde_json::Value>)> {
    let file_path = sessions_dir()?.join(format!("{}.json", session_id));
    let data = tokio::fs::read_to_string(&file_path)
        .await
        .with_context(|| format!("failed to read session file: {}", file_path.display()))?;
    let session_file: SessionFile =
        serde_json::from_str(&data).context("failed to parse session file")?;

    Ok((session_file.meta, session_file.messages))
}

/// List all saved sessions, sorted by updated_at descending
pub async fn list_sessions() -> Result<Vec<SessionMeta>> {
    let dir = sessions_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = tokio::fs::read_dir(&dir).await?;
    let mut sessions = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match tokio::fs::read_to_string(&path).await {
            Ok(data) => {
                if let Ok(session_file) = serde_json::from_str::<SessionFile>(&data) {
                    sessions.push(session_file.meta);
                }
            }
            Err(_) => continue,
        }
    }

    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

/// Delete a session file
pub async fn delete_session(session_id: &str) -> Result<()> {
    let file_path = sessions_dir()?.join(format!("{}.json", session_id));
    tokio::fs::remove_file(&file_path)
        .await
        .with_context(|| format!("failed to delete session: {}", file_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sessions_dir() {
        let dir = sessions_dir().unwrap();
        assert!(dir.ends_with(".venus/sessions"));
    }

    #[test]
    fn test_session_meta_serialization() {
        let meta = SessionMeta {
            id: "test-id".to_string(),
            project: "/tmp/test".to_string(),
            created_at: 1000,
            updated_at: 2000,
            message_count: 5,
            model: "venus-3".to_string(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: SessionMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "test-id");
        assert_eq!(deserialized.message_count, 5);
    }
}
