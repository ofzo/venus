use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_read_per_million: f64,
    pub cache_write_per_million: f64,
}

fn default_pricing() -> HashMap<&'static str, ModelPricing> {
    let mut m = HashMap::new();
    m.insert(
        "claude-sonnet-4-20250514",
        ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
            cache_read_per_million: 0.3,
            cache_write_per_million: 3.75,
        },
    );
    m.insert(
        "claude-opus-4-20250514",
        ModelPricing {
            input_per_million: 15.0,
            output_per_million: 75.0,
            cache_read_per_million: 1.5,
            cache_write_per_million: 18.75,
        },
    );
    m.insert(
        "claude-haiku-4-20250506",
        ModelPricing {
            input_per_million: 0.80,
            output_per_million: 4.0,
            cache_read_per_million: 0.08,
            cache_write_per_million: 1.0,
        },
    );
    m
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}

impl TokenUsage {
    pub fn add(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_creation_tokens
    }
}

#[derive(Debug, Clone, Default)]
pub struct CostTracker {
    pub usage_by_model: HashMap<String, TokenUsage>,
}

impl CostTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, model: &str, usage: &TokenUsage) {
        self.usage_by_model
            .entry(model.to_string())
            .or_default()
            .add(usage);
    }

    pub fn total_cost_usd(&self) -> f64 {
        let pricing = default_pricing();
        let mut total = 0.0;

        for (model, usage) in &self.usage_by_model {
            if let Some(price) = pricing.get(model.as_str()) {
                total += usage.input_tokens as f64 * price.input_per_million / 1_000_000.0;
                total += usage.output_tokens as f64 * price.output_per_million / 1_000_000.0;
                total +=
                    usage.cache_read_tokens as f64 * price.cache_read_per_million / 1_000_000.0;
                total += usage.cache_creation_tokens as f64 * price.cache_write_per_million
                    / 1_000_000.0;
            } else {
                // Fallback: use Sonnet pricing
                let fallback = &pricing["claude-sonnet-4-20250514"];
                total += usage.input_tokens as f64 * fallback.input_per_million / 1_000_000.0;
                total += usage.output_tokens as f64 * fallback.output_per_million / 1_000_000.0;
                total += usage.cache_read_tokens as f64 * fallback.cache_read_per_million
                    / 1_000_000.0;
                total += usage.cache_creation_tokens as f64 * fallback.cache_write_per_million
                    / 1_000_000.0;
            }
        }

        total
    }

    pub fn total_usage(&self) -> TokenUsage {
        let mut total = TokenUsage::default();
        for usage in self.usage_by_model.values() {
            total.add(usage);
        }
        total
    }

    pub fn format_cost(&self) -> String {
        let cost = self.total_cost_usd();
        if cost < 0.01 {
            format!("${:.4}", cost)
        } else {
            format!("${:.2}", cost)
        }
    }

    pub fn format_tokens(&self) -> String {
        let total = self.total_usage();
        format!(
            "{}in/{}out",
            format_token_count(total.input_tokens + total.cache_read_tokens),
            format_token_count(total.output_tokens)
        )
    }
}

fn format_token_count(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_tracking() {
        let mut tracker = CostTracker::new();
        tracker.record(
            "claude-sonnet-4-20250514",
            &TokenUsage {
                input_tokens: 1000,
                output_tokens: 500,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
        );
        let cost = tracker.total_cost_usd();
        // 1000 * 3.0/1M + 500 * 15.0/1M = 0.003 + 0.0075 = 0.0105
        assert!((cost - 0.0105).abs() < 0.0001);
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_token_count(500), "500");
        assert_eq!(format_token_count(1500), "1.5K");
        assert_eq!(format_token_count(1_500_000), "1.5M");
    }
}
