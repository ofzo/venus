use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionMode {
    Default,
    Auto,
    Plan,
    Bypass,
}

impl PermissionMode {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "bypass" | "yolo" => Self::Bypass,
            "plan" => Self::Plan,
            "auto" => Self::Auto,
            _ => Self::Default,
        }
    }
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::Default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_modes() {
        assert_eq!(PermissionMode::from_str("default"), PermissionMode::Default);
        assert_eq!(PermissionMode::from_str("bypass"), PermissionMode::Bypass);
        assert_eq!(PermissionMode::from_str("yolo"), PermissionMode::Bypass);
        assert_eq!(PermissionMode::from_str("plan"), PermissionMode::Plan);
        assert_eq!(PermissionMode::from_str("auto"), PermissionMode::Auto);
        assert_eq!(PermissionMode::from_str("unknown"), PermissionMode::Default);
        assert_eq!(PermissionMode::from_str("BYPASS"), PermissionMode::Bypass);
    }
}
