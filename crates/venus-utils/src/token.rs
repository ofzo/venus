/// Rough token estimation: ~4 characters per token for English text.
/// This is a heuristic and not exact, but sufficient for budget tracking.
pub fn estimate_tokens(text: &str) -> u64 {
    (text.len() as f64 / 4.0).ceil() as u64
}

/// Estimate tokens for a JSON value (serialized size / 4).
pub fn estimate_tokens_json(value: &serde_json::Value) -> u64 {
    let text = serde_json::to_string(value).unwrap_or_default();
    estimate_tokens(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars / 4 = 2.75 -> 3
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
    }
}
