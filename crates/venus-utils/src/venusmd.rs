use anyhow::Result;
use std::path::Path;
use tracing::debug;

use crate::fs_helpers;

#[derive(Debug, Clone)]
pub struct VenusMdFile {
    pub source: VenusMdSource,
    pub content: String,
}

#[derive(Debug, Clone)]
pub enum VenusMdSource {
    Global,
    User,
    Project,
}

/// Discover and load all VENUS.md (and legacy CLAUDE.md) files from global, user, and project paths.
pub async fn load_venus_md_files(project_root: Option<&Path>) -> Result<Vec<VenusMdFile>> {
    let mut files = Vec::new();

    // 1. User VENUS.md: ~/.venus/VENUS.md
    if let Ok(config_dir) = fs_helpers::venus_config_dir() {
        let user_path = config_dir.join("VENUS.md");
        if user_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&user_path).await {
                debug!("loaded user VENUS.md from {:?}", user_path);
                files.push(VenusMdFile {
                    source: VenusMdSource::User,
                    content,
                });
            }
        }
    }

    // 2. Project VENUS.md files
    if let Some(root) = project_root {
        // Check VENUS.md at project root
        let project_path = root.join("VENUS.md");
        if project_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&project_path).await {
                debug!("loaded project VENUS.md from {:?}", project_path);
                files.push(VenusMdFile {
                    source: VenusMdSource::Project,
                    content,
                });
            }
        }

        // Check .venus/VENUS.md
        let dot_venus_path = root.join(".venus").join("VENUS.md");
        if dot_venus_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&dot_venus_path).await {
                debug!("loaded .venus/VENUS.md from {:?}", dot_venus_path);
                files.push(VenusMdFile {
                    source: VenusMdSource::Project,
                    content,
                });
            }
        }
    }

    Ok(files)
}

/// Merge all VENUS.md files into a single system prompt string.
pub fn merge_venus_md(files: &[VenusMdFile]) -> String {
    if files.is_empty() {
        return String::new();
    }

    files
        .iter()
        .map(|f| {
            let source_label = match f.source {
                VenusMdSource::Global => "Global",
                VenusMdSource::User => "User",
                VenusMdSource::Project => "Project",
            };
            format!("# {} Instructions\n\n{}", source_label, f.content.trim())
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}
