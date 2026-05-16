use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("could not determine home directory")
}

pub fn claude_config_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".venus"))
}

pub fn venus_config_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".venus"))
}

pub fn venus_global_config_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".venus.json"))
}

pub fn resolve_path(path: &str, working_dir: &Path) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        working_dir.join(p)
    }
}

pub fn is_text_file(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => !matches!(
            ext.to_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "webp" | "svg"
                | "mp3" | "mp4" | "avi" | "mov" | "wav"
                | "zip" | "tar" | "gz" | "bz2" | "xz" | "7z"
                | "exe" | "dll" | "so" | "dylib"
                | "bin" | "dat" | "db" | "sqlite"
                | "woff" | "woff2" | "ttf" | "otf" | "eot"
        ),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_path_absolute() {
        let result = resolve_path("/tmp/test.txt", Path::new("/home/user"));
        assert_eq!(result, PathBuf::from("/tmp/test.txt"));
    }

    #[test]
    fn test_resolve_path_relative() {
        let result = resolve_path("src/main.rs", Path::new("/home/user/project"));
        assert_eq!(result, PathBuf::from("/home/user/project/src/main.rs"));
    }

    #[test]
    fn test_is_text_file() {
        assert!(is_text_file(Path::new("test.rs")));
        assert!(is_text_file(Path::new("test.txt")));
        assert!(!is_text_file(Path::new("test.png")));
        assert!(!is_text_file(Path::new("test.exe")));
    }
}
