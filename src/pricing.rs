use crate::model::Usage;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// $ / 1M tokens 単位の単価。
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ModelPricing {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_write: Option<f64>,
    #[serde(default)]
    pub cache_read: Option<f64>,
}

impl ModelPricing {
    fn cache_write_rate(&self) -> f64 {
        self.cache_write.unwrap_or(self.input * 1.25)
    }
    fn cache_read_rate(&self) -> f64 {
        self.cache_read.unwrap_or(self.input * 0.1)
    }
}

/// モデル名の部分一致パターン → 単価。先頭から順に最初にマッチしたものを採用。
pub struct PricingTable {
    entries: Vec<(String, ModelPricing)>,
}

#[derive(Debug, Deserialize, Default)]
struct PricingFile {
    #[serde(default)]
    models: HashMap<String, ModelPricing>,
}

fn embedded_defaults() -> Vec<(String, ModelPricing)> {
    vec![
        // Anthropic
        (
            "claude-opus".into(),
            ModelPricing { input: 15.0, output: 75.0, cache_write: None, cache_read: None },
        ),
        (
            "claude-sonnet".into(),
            ModelPricing { input: 3.0, output: 15.0, cache_write: None, cache_read: None },
        ),
        (
            "claude-haiku".into(),
            ModelPricing { input: 0.8, output: 4.0, cache_write: None, cache_read: None },
        ),
        (
            "claude-fable".into(),
            ModelPricing { input: 3.0, output: 15.0, cache_write: None, cache_read: None },
        ),
        // OpenAI (Codex)
        (
            "gpt-5".into(),
            ModelPricing { input: 5.0, output: 15.0, cache_write: Some(5.0), cache_read: Some(0.5) },
        ),
        (
            "gpt-4.1".into(),
            ModelPricing { input: 2.0, output: 8.0, cache_write: Some(2.0), cache_read: Some(0.5) },
        ),
        (
            "gpt-4o".into(),
            ModelPricing { input: 2.5, output: 10.0, cache_write: Some(2.5), cache_read: Some(1.25) },
        ),
        (
            "o3".into(),
            ModelPricing { input: 2.0, output: 8.0, cache_write: None, cache_read: None },
        ),
        (
            "o4-mini".into(),
            ModelPricing { input: 1.1, output: 4.4, cache_write: None, cache_read: None },
        ),
    ]
}

impl PricingTable {
    pub fn load(override_path: Option<PathBuf>) -> Self {
        let mut entries = embedded_defaults();

        let path = override_path.unwrap_or_else(default_override_path);
        if let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(parsed) = toml::from_str::<PricingFile>(&content) {
                for (pattern, pricing) in parsed.models {
                    if let Some(existing) = entries.iter_mut().find(|(p, _)| *p == pattern) {
                        existing.1 = pricing;
                    } else {
                        // ユーザー定義は優先して先頭に挿入(埋め込みデフォルトより先にマッチさせる)
                        entries.insert(0, (pattern, pricing));
                    }
                }
            }

        PricingTable { entries }
    }

    fn lookup(&self, model: &str) -> Option<&ModelPricing> {
        let lower = model.to_lowercase();
        self.entries
            .iter()
            .find(|(pattern, _)| lower.contains(pattern.as_str()))
            .map(|(_, p)| p)
    }

    /// 既知モデルならコストを、未知モデルなら None を返す。
    pub fn calculate(&self, model: &str, usage: &Usage) -> Option<f64> {
        let pricing = self.lookup(model)?;
        let mtok = 1_000_000.0;
        let cost = usage.input_tokens as f64 / mtok * pricing.input
            + usage.output_tokens as f64 / mtok * pricing.output
            + usage.cache_creation_tokens as f64 / mtok * pricing.cache_write_rate()
            + usage.cache_read_tokens as f64 / mtok * pricing.cache_read_rate();
        Some(cost)
    }

}

fn default_override_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("llmeter")
        .join("pricing.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_matches_by_substring() {
        let table = PricingTable { entries: embedded_defaults() };
        let usage = Usage { input_tokens: 1_000_000, output_tokens: 1_000_000, ..Default::default() };
        let cost = table.calculate("claude-sonnet-5-20260115", &usage).unwrap();
        assert!((cost - 18.0).abs() < 1e-9);
    }

    #[test]
    fn unknown_model_returns_none() {
        let table = PricingTable { entries: embedded_defaults() };
        let usage = Usage::default();
        assert!(table.calculate("some-future-model", &usage).is_none());
    }

    #[test]
    fn cache_rates_default_relative_to_input() {
        let table = PricingTable { entries: embedded_defaults() };
        let usage = Usage {
            cache_creation_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
            ..Default::default()
        };
        // claude-sonnet: input 3.0 -> cache_write 3.75, cache_read 0.3
        let cost = table.calculate("claude-sonnet-5", &usage).unwrap();
        assert!((cost - 4.05).abs() < 1e-9);
    }
}
