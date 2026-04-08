/// Default context window size for most models.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;

/// Tokens reserved for the compaction summary output.
pub const COMPACT_RESERVED_OUTPUT: u64 = 20_000;

/// Buffer between the effective window and the auto-compact trigger point.
pub const AUTO_COMPACT_BUFFER: u64 = 13_000;

/// Returns the context window size (in tokens) for a given model.
pub fn context_window_for_model(model: &str) -> u64 {
    // Extended context models (1M)
    if model.contains("[1m]") || model.contains("extended") {
        return 1_000_000;
    }

    // Model-specific windows
    if model.contains("opus-4-6") || model.contains("opus-4-5") {
        return 200_000;
    }
    if model.contains("sonnet-4-6") || model.contains("sonnet-4") {
        return 200_000;
    }
    if model.contains("haiku") {
        return 200_000;
    }

    DEFAULT_CONTEXT_WINDOW
}

/// Returns the maximum output tokens for a given model.
pub fn max_output_for_model(model: &str) -> u32 {
    if model.contains("opus-4-6") {
        return 64_000;
    }
    if model.contains("sonnet-4-6") {
        return 32_000;
    }
    if model.contains("opus-4-5") || model.contains("opus-4") {
        return 32_000;
    }
    if model.contains("sonnet-4") {
        return 32_000;
    }
    if model.contains("haiku") {
        return 16_384;
    }
    16_384
}

/// Returns the token count at which auto-compact should trigger.
///
/// Formula: context_window - min(max_output, COMPACT_RESERVED_OUTPUT) - AUTO_COMPACT_BUFFER
pub fn auto_compact_threshold(model: &str) -> u64 {
    let window = context_window_for_model(model);
    let max_out = max_output_for_model(model) as u64;
    let reserved = max_out.min(COMPACT_RESERVED_OUTPUT);
    window.saturating_sub(reserved).saturating_sub(AUTO_COMPACT_BUFFER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_context_window() {
        assert_eq!(context_window_for_model("unknown-model"), DEFAULT_CONTEXT_WINDOW);
    }

    #[test]
    fn test_extended_context() {
        assert_eq!(context_window_for_model("claude-sonnet-4-6[1m]"), 1_000_000);
    }

    #[test]
    fn test_auto_compact_threshold_default() {
        let threshold = auto_compact_threshold("claude-haiku-4-20250506");
        // 200_000 - 16_384 - 13_000 = 170_616
        assert_eq!(threshold, 200_000 - 16_384 - 13_000);
    }

    #[test]
    fn test_auto_compact_threshold_opus() {
        let threshold = auto_compact_threshold("claude-opus-4-6-20250514");
        // 200_000 - 20_000 (capped) - 13_000 = 167_000
        assert_eq!(threshold, 167_000);
    }
}
