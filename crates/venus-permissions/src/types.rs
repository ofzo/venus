use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionMode {
    #[default]
    Default,
    Auto,
    Plan,
    Bypass,
}

impl PermissionMode {
    pub fn parse_mode(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "bypass" | "yolo" => Self::Bypass,
            "plan" => Self::Plan,
            "auto" => Self::Auto,
            _ => Self::Default,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_modes() {
        assert_eq!(PermissionMode::parse_mode("default"), PermissionMode::Default);
        assert_eq!(PermissionMode::parse_mode("bypass"), PermissionMode::Bypass);
        assert_eq!(PermissionMode::parse_mode("yolo"), PermissionMode::Bypass);
        assert_eq!(PermissionMode::parse_mode("plan"), PermissionMode::Plan);
        assert_eq!(PermissionMode::parse_mode("auto"), PermissionMode::Auto);
        assert_eq!(PermissionMode::parse_mode("unknown"), PermissionMode::Default);
        assert_eq!(PermissionMode::parse_mode("BYPASS"), PermissionMode::Bypass);
    }
}
