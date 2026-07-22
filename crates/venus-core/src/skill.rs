use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// A reusable prompt template loaded from a markdown file with YAML frontmatter.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub user_invocable: bool,
    pub source_path: PathBuf,
}

/// Registry that holds all loaded skills and provides lookup.
pub struct SkillRegistry {
    skills: Vec<Skill>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self { skills: Vec::new() }
    }

    /// Load skills from multiple directories, merging results.
    /// Later directories take precedence (skills with the same name are overwritten).
    pub async fn load_from_dirs(dirs: &[PathBuf]) -> Result<Self> {
        let mut registry = Self::new();

        for dir in dirs {
            if !dir.is_dir() {
                debug!("skill directory does not exist: {}", dir.display());
                continue;
            }

            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(e) => {
                    warn!("failed to read skill directory {}: {}", dir.display(), e);
                    continue;
                }
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }

                match parse_skill_file(&path) {
                    Ok(skill) => {
                        debug!("loaded skill '{}' from {}", skill.name, path.display());
                        // Remove existing skill with same name (later dirs override)
                        registry
                            .skills
                            .retain(|s| s.name.to_lowercase() != skill.name.to_lowercase());
                        registry.skills.push(skill);
                    }
                    Err(e) => {
                        warn!("failed to parse skill file {}: {}", path.display(), e);
                    }
                }
            }
        }

        Ok(registry)
    }

    /// Find a skill by name (case-insensitive).
    pub fn find(&self, name: &str) -> Option<&Skill> {
        let name_lower = name.to_lowercase();
        self.skills
            .iter()
            .find(|s| s.name.to_lowercase() == name_lower)
    }

    /// List all user-invocable skills.
    pub fn user_invocable(&self) -> Vec<&Skill> {
        self.skills.iter().filter(|s| s.user_invocable).collect()
    }

    /// List all skills.
    pub fn all(&self) -> &[Skill] {
        &self.skills
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a skill from a markdown file with YAML frontmatter.
fn parse_skill_file(path: &Path) -> Result<Skill> {
    let content = std::fs::read_to_string(path)?;

    if !content.starts_with("---") {
        return Err(anyhow::anyhow!("skill file missing frontmatter"));
    }

    let end = content[3..]
        .find("---")
        .ok_or_else(|| anyhow::anyhow!("skill file missing closing frontmatter"))?;

    let frontmatter = &content[3..end + 3];
    let body = content[end + 6..].trim().to_string();

    let mut name = String::new();
    let mut description = String::new();
    let mut user_invocable = false;

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("user_invocable:") {
            user_invocable = val.trim() == "true";
        }
    }

    if name.is_empty() {
        name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
    }

    Ok(Skill {
        name,
        description,
        content: body,
        user_invocable,
        source_path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_skill(dir: &Path, filename: &str, content: &str) {
        let path = dir.join(filename);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[tokio::test]
    async fn test_load_from_dirs() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "commit.md",
            "---\nname: commit\ndescription: Create a commit\nuser_invocable: true\n---\n\nCommit skill body.",
        );
        write_skill(
            tmp.path(),
            "review.md",
            "---\nname: review\ndescription: Code review\nuser_invocable: false\n---\n\nReview body.",
        );

        let registry = SkillRegistry::load_from_dirs(&[tmp.path().to_path_buf()])
            .await
            .unwrap();

        assert_eq!(registry.all().len(), 2);
        assert_eq!(registry.user_invocable().len(), 1);

        let commit = registry.find("commit").unwrap();
        assert_eq!(commit.description, "Create a commit");
        assert!(commit.user_invocable);
        assert_eq!(commit.content, "Commit skill body.");

        // Case insensitive
        assert!(registry.find("COMMIT").is_some());
        assert!(registry.find("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_name_fallback_to_filename() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "deploy.md",
            "---\ndescription: Deploy things\n---\n\nDeploy body.",
        );

        let registry = SkillRegistry::load_from_dirs(&[tmp.path().to_path_buf()])
            .await
            .unwrap();

        let skill = registry.find("deploy").unwrap();
        assert_eq!(skill.name, "deploy");
    }

    #[tokio::test]
    async fn test_override_by_later_dir() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();
        write_skill(
            dir1.path(),
            "commit.md",
            "---\nname: commit\ndescription: V1\n---\n\nV1 body.",
        );
        write_skill(
            dir2.path(),
            "commit.md",
            "---\nname: commit\ndescription: V2\n---\n\nV2 body.",
        );

        let registry =
            SkillRegistry::load_from_dirs(&[dir1.path().to_path_buf(), dir2.path().to_path_buf()])
                .await
                .unwrap();

        let commit = registry.find("commit").unwrap();
        assert_eq!(commit.description, "V2");
    }

    #[tokio::test]
    async fn test_missing_dir_is_ok() {
        let registry = SkillRegistry::load_from_dirs(&[PathBuf::from("/nonexistent/path/skills")])
            .await
            .unwrap();
        assert!(registry.all().is_empty());
    }
}
