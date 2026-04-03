use anyhow::Result;
use std::path::Path;
use tracing::debug;

use crate::fs_helpers;

#[derive(Debug, Clone)]
pub struct ClaudeMdFile {
    pub source: ClaudeMdSource,
    pub content: String,
}

#[derive(Debug, Clone)]
pub enum ClaudeMdSource {
    Global,
    User,
    Project,
}

/// Discover and load all CLAUDE.md files from global, user, and project paths.
pub async fn load_claude_md_files(project_root: Option<&Path>) -> Result<Vec<ClaudeMdFile>> {
    let mut files = Vec::new();

    // 1. Global managed settings: /etc/claude-code/CLAUDE.md
    let global_path = Path::new("/etc/claude-code/CLAUDE.md");
    if global_path.exists() {
        if let Ok(content) = tokio::fs::read_to_string(global_path).await {
            debug!("loaded global CLAUDE.md");
            files.push(ClaudeMdFile {
                source: ClaudeMdSource::Global,
                content,
            });
        }
    }

    // 2. User CLAUDE.md: ~/.claude/CLAUDE.md
    if let Ok(config_dir) = fs_helpers::claude_config_dir() {
        let user_path = config_dir.join("CLAUDE.md");
        if user_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&user_path).await {
                debug!("loaded user CLAUDE.md from {:?}", user_path);
                files.push(ClaudeMdFile {
                    source: ClaudeMdSource::User,
                    content,
                });
            }
        }
    }

    // 3. Project CLAUDE.md files
    if let Some(root) = project_root {
        // Check CLAUDE.md at project root
        let project_path = root.join("CLAUDE.md");
        if project_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&project_path).await {
                debug!("loaded project CLAUDE.md from {:?}", project_path);
                files.push(ClaudeMdFile {
                    source: ClaudeMdSource::Project,
                    content,
                });
            }
        }

        // Check .claude/CLAUDE.md
        let dot_claude_path = root.join(".claude").join("CLAUDE.md");
        if dot_claude_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&dot_claude_path).await {
                debug!("loaded .claude/CLAUDE.md from {:?}", dot_claude_path);
                files.push(ClaudeMdFile {
                    source: ClaudeMdSource::Project,
                    content,
                });
            }
        }
    }

    Ok(files)
}

/// Merge all CLAUDE.md files into a single system prompt string.
pub fn merge_claude_md(files: &[ClaudeMdFile]) -> String {
    if files.is_empty() {
        return String::new();
    }

    files
        .iter()
        .map(|f| {
            let source_label = match f.source {
                ClaudeMdSource::Global => "Global",
                ClaudeMdSource::User => "User",
                ClaudeMdSource::Project => "Project",
            };
            format!("# {} Instructions\n\n{}", source_label, f.content.trim())
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}
