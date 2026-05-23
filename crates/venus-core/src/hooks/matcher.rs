use super::events::HookEvent;
use venus_utils::config::HookEntry;

/// Filter hook entries that match the given event.
pub fn matching_hooks<'a>(entries: &'a [HookEntry], event: &HookEvent) -> Vec<&'a HookEntry> {
    entries
        .iter()
        .filter(|entry| matches_event(entry, event))
        .collect()
}

fn matches_event(entry: &HookEntry, event: &HookEvent) -> bool {
    match &entry.matcher {
        None => true,
        Some(pattern) => {
            if let Some(tool_name) = event.tool_name() {
                glob_match(pattern, tool_name)
            } else {
                // Non-tool events ignore matcher
                true
            }
        }
    }
}

/// Simple glob matching: "*" matches all, "Bash*" prefix match, exact match.
fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(stripped) = pattern.strip_suffix('*') {
        name.starts_with(stripped)
    } else {
        pattern == name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("Bash", "Bash"));
        assert!(!glob_match("Bash", "BashTerminal"));
    }

    #[test]
    fn test_glob_match_wildcard() {
        assert!(glob_match("*", "Bash"));
        assert!(glob_match("*", "Read"));
    }

    #[test]
    fn test_glob_match_prefix() {
        assert!(glob_match("Bash*", "Bash"));
        assert!(glob_match("Bash*", "BashTerminal"));
        assert!(!glob_match("Bash*", "Read"));
    }
}
