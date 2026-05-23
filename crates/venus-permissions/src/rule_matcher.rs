use regex::Regex;
use serde_json::Value;

use crate::rule_parser::ParsedRule;

/// Check if a rule matches the given tool name and input.
pub fn rule_matches(rule: &ParsedRule, tool_name: &str, input: &Value) -> bool {
    if !tool_name_matches(&rule.tool, tool_name) {
        return false;
    }

    let pattern = match &rule.pattern {
        None => return true,
        Some(p) => p,
    };

    let content = extract_matchable_content(tool_name, input);
    wildcard_match(pattern, &content)
}

/// Match tool name: exact or trailing wildcard (`Bash*` matches `Bash`, `BashTerminal`).
fn tool_name_matches(rule_tool: &str, actual: &str) -> bool {
    if rule_tool == actual {
        return true;
    }
    if let Some(stripped) = rule_tool.strip_suffix('*') {
        actual.starts_with(stripped)
    } else {
        false
    }
}

/// Extract the matchable content string from tool input based on tool type.
fn extract_matchable_content(tool_name: &str, input: &Value) -> String {
    match tool_name {
        "Bash" => input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Read" | "Write" | "Edit" => input
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Glob" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Grep" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => serde_json::to_string(input).unwrap_or_default(),
    }
}

/// Wildcard pattern matching inspired by TS `matchWildcardPattern`.
///
/// `*` matches any sequence of characters.
/// Trailing ` *` (space+star) is optional, so `git *` matches both `git` and `git status`.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let has_optional_tail = pattern.ends_with(" *");

    let base = if has_optional_tail {
        &pattern[..pattern.len() - 2]
    } else {
        pattern
    };

    let mut regex_str = String::from("^");
    for ch in base.chars() {
        match ch {
            '*' => regex_str.push_str(".*"),
            '.' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '\\' | '^' | '$' | '|' => {
                regex_str.push('\\');
                regex_str.push(ch);
            }
            _ => regex_str.push(ch),
        }
    }

    if has_optional_tail {
        regex_str.push_str("( .*)?");
    }
    regex_str.push('$');

    Regex::new(&regex_str)
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wildcard_git_star() {
        assert!(wildcard_match("git *", "git"));
        assert!(wildcard_match("git *", "git status"));
        assert!(wildcard_match("git *", "git push origin main"));
        assert!(!wildcard_match("git *", "npm install"));
        assert!(!wildcard_match("git *", "gitignore"));
    }

    #[test]
    fn wildcard_glob_star() {
        assert!(wildcard_match("src/*", "src/foo.rs"));
        assert!(wildcard_match("src/*", "src/a/b.rs"));
        assert!(!wildcard_match("src/*", "lib/foo.rs"));
    }

    #[test]
    fn wildcard_double_star() {
        assert!(wildcard_match("src/**", "src/foo.rs"));
        assert!(wildcard_match("src/**", "src/a/b/c.rs"));
    }

    #[test]
    fn exact_match() {
        assert!(wildcard_match("npm install", "npm install"));
        assert!(!wildcard_match("npm install", "npm install --save"));
        assert!(!wildcard_match("npm install", "npm test"));
    }

    #[test]
    fn star_matches_all() {
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("*", "git status"));
        assert!(wildcard_match("*", ""));
    }

    #[test]
    fn tool_name_exact() {
        assert!(tool_name_matches("Bash", "Bash"));
        assert!(!tool_name_matches("Bash", "Read"));
    }

    #[test]
    fn tool_name_wildcard() {
        assert!(tool_name_matches("Bash*", "Bash"));
        assert!(tool_name_matches("Bash*", "BashTerminal"));
        assert!(!tool_name_matches("Bash*", "Read"));
    }

    #[test]
    fn rule_matches_bash_git() {
        let rule = ParsedRule {
            tool: "Bash".to_string(),
            pattern: Some("git *".to_string()),
        };
        let input = json!({"command": "git status"});
        assert!(rule_matches(&rule, "Bash", &input));

        let input2 = json!({"command": "rm -rf /"});
        assert!(!rule_matches(&rule, "Bash", &input2));
    }

    #[test]
    fn rule_matches_tool_only() {
        let rule = ParsedRule {
            tool: "Read".to_string(),
            pattern: None,
        };
        let input = json!({"file_path": "/any/path"});
        assert!(rule_matches(&rule, "Read", &input));
        assert!(!rule_matches(&rule, "Write", &input));
    }

    #[test]
    fn rule_matches_write_pattern() {
        let rule = ParsedRule {
            tool: "Write".to_string(),
            pattern: Some("src/*".to_string()),
        };
        let input = json!({"file_path": "src/main.rs"});
        assert!(rule_matches(&rule, "Write", &input));

        let input2 = json!({"file_path": "lib/main.rs"});
        assert!(!rule_matches(&rule, "Write", &input2));
    }

    #[test]
    fn extract_bash_command() {
        let input = json!({"command": "ls -la"});
        assert_eq!(extract_matchable_content("Bash", &input), "ls -la");
    }

    #[test]
    fn extract_file_path() {
        let input = json!({"file_path": "/tmp/foo.txt"});
        assert_eq!(extract_matchable_content("Read", &input), "/tmp/foo.txt");
        assert_eq!(extract_matchable_content("Write", &input), "/tmp/foo.txt");
        assert_eq!(extract_matchable_content("Edit", &input), "/tmp/foo.txt");
    }
}
