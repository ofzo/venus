use venus_utils::config::PermissionRule;

/// A parsed permission rule: tool name + optional content pattern.
#[derive(Debug, Clone)]
pub struct ParsedRule {
    pub tool: String,
    pub pattern: Option<String>,
}

/// Parse a PermissionRule from config into a ParsedRule.
///
/// `PermissionRule` already has separate `tool` and `pattern` fields,
/// so this is a straightforward conversion with validation.
pub fn parse_rule(rule: &PermissionRule) -> ParsedRule {
    ParsedRule {
        tool: rule.tool.clone(),
        pattern: rule.pattern.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_only() {
        let rule = PermissionRule {
            tool: "Bash".to_string(),
            pattern: None,
        };
        let parsed = parse_rule(&rule);
        assert_eq!(parsed.tool, "Bash");
        assert!(parsed.pattern.is_none());
    }

    #[test]
    fn parse_tool_with_pattern() {
        let rule = PermissionRule {
            tool: "Bash".to_string(),
            pattern: Some("git *".to_string()),
        };
        let parsed = parse_rule(&rule);
        assert_eq!(parsed.tool, "Bash");
        assert_eq!(parsed.pattern.as_deref(), Some("git *"));
    }

    #[test]
    fn parse_file_path_pattern() {
        let rule = PermissionRule {
            tool: "Write".to_string(),
            pattern: Some("src/**".to_string()),
        };
        let parsed = parse_rule(&rule);
        assert_eq!(parsed.tool, "Write");
        assert_eq!(parsed.pattern.as_deref(), Some("src/**"));
    }
}
