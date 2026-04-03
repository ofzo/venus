use anyhow::Result;
use std::path::Path;
use tokio::process::Command;

pub async fn find_git_root(cwd: &Path) -> Result<Option<std::path::PathBuf>> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .await?;

    if output.status.success() {
        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Some(std::path::PathBuf::from(root)))
    } else {
        Ok(None)
    }
}

pub async fn is_git_repo(cwd: &Path) -> bool {
    find_git_root(cwd).await.ok().flatten().is_some()
}

pub async fn git_branch(cwd: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(cwd)
        .output()
        .await?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn git_status(cwd: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .output()
        .await?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn git_log(cwd: &Path, count: usize) -> Result<String> {
    let output = Command::new("git")
        .args([
            "log",
            "--oneline",
            &format!("-{}", count),
            "--no-decorate",
        ])
        .current_dir(cwd)
        .output()
        .await?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn git_diff(cwd: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["diff", "--stat"])
        .current_dir(cwd)
        .output()
        .await?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[derive(Debug, Clone)]
pub struct GitContext {
    pub branch: String,
    pub status: String,
    pub recent_log: String,
}

pub async fn get_git_context(cwd: &Path) -> Result<Option<GitContext>> {
    if !is_git_repo(cwd).await {
        return Ok(None);
    }

    let (branch, status, log) =
        tokio::try_join!(git_branch(cwd), git_status(cwd), git_log(cwd, 5))?;

    Ok(Some(GitContext {
        branch,
        status,
        recent_log: log,
    }))
}
