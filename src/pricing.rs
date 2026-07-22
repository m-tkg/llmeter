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
    pub fn cache_write_rate(&self) -> f64 {
        self.cache_write.unwrap_or(self.input * 1.25)
    }
    pub fn cache_read_rate(&self) -> f64 {
        self.cache_read.unwrap_or(self.input * 0.1)
    }
}

/// 単価がどの層から採用されたか（`pricing show` の表示用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricingSource {
    /// pricing.toml によるユーザー上書き
    User,
    /// LiteLLM 料金データベース
    LiteLlm,
    /// バイナリ内蔵の埋め込みデフォルト
    Embedded,
}

impl PricingSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            PricingSource::User => "pricing.toml（ユーザー上書き）",
            PricingSource::LiteLlm => "LiteLLM 料金データベース",
            PricingSource::Embedded => "埋め込みデフォルト",
        }
    }
}

/// 料金解決の3層構造:
/// 1. pricing.toml（ユーザー上書き、部分一致、最優先）
/// 2. LiteLLM データ（完全一致 → 接頭辞除去完全一致 → 最長プレフィックス一致）
/// 3. 埋め込みデフォルト（部分一致、fallback）
pub struct PricingTable {
    user_entries: Vec<(String, ModelPricing)>,
    litellm: HashMap<String, ModelPricing>,
    embedded_entries: Vec<(String, ModelPricing)>,
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

/// LiteLLM キーの `provider/` `provider.` 接頭辞（先頭セグメントのみ）を除去する。
fn strip_provider_prefix(key: &str) -> &str {
    match key.find(['/', '.']) {
        Some(idx) => &key[idx + 1..],
        None => key,
    }
}

impl PricingTable {
    pub fn load(override_path: Option<PathBuf>, offline: bool) -> Self {
        let mut user_entries = Vec::new();
        let path = override_path.unwrap_or_else(default_override_path);
        if let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(parsed) = toml::from_str::<PricingFile>(&content)
        {
            for (pattern, pricing) in parsed.models {
                user_entries.push((pattern, pricing));
            }
        }

        let litellm = crate::litellm::load(offline).unwrap_or_default();

        PricingTable { user_entries, litellm, embedded_entries: embedded_defaults() }
    }

    /// モデル名を解決し、採用層と単価を返す。`llmeter pricing show` でも使う。
    pub fn resolve(&self, model: &str) -> Option<(PricingSource, ModelPricing)> {
        let lower = model.to_lowercase();

        if let Some((_, p)) = self.user_entries.iter().find(|(pat, _)| lower.contains(pat.as_str())) {
            return Some((PricingSource::User, *p));
        }

        if let Some(p) = self.litellm.get(&lower) {
            return Some((PricingSource::LiteLlm, *p));
        }

        if let Some((_, p)) = self
            .litellm
            .iter()
            .find(|(key, _)| strip_provider_prefix(key) == lower)
        {
            return Some((PricingSource::LiteLlm, *p));
        }

        if let Some(p) = self.longest_prefix_match(&lower) {
            return Some((PricingSource::LiteLlm, p));
        }

        if let Some((_, p)) = self.embedded_entries.iter().find(|(pat, _)| lower.contains(pat.as_str())) {
            return Some((PricingSource::Embedded, *p));
        }

        None
    }

    /// ログのモデル名と LiteLLM キーのうち短い方が長い方の接頭辞になっている場合、
    /// 一致長が最大のエントリを返す（8文字未満は誤マッチ防止のため不採用）。
    fn longest_prefix_match(&self, lower_model: &str) -> Option<ModelPricing> {
        let mut best: Option<(usize, ModelPricing)> = None;
        for (key, pricing) in &self.litellm {
            let match_len = if lower_model.starts_with(key.as_str()) {
                key.len()
            } else if key.starts_with(lower_model) {
                lower_model.len()
            } else {
                0
            };
            if match_len >= 8 && best.as_ref().is_none_or(|(best_len, _)| match_len > *best_len) {
                best = Some((match_len, *pricing));
            }
        }
        best.map(|(_, p)| p)
    }

    /// 既知モデルならコストを、未知モデルなら None を返す。
    pub fn calculate(&self, model: &str, usage: &Usage) -> Option<f64> {
        let (_, pricing) = self.resolve(model)?;
        let mtok = 1_000_000.0;
        let cost = usage.input_tokens as f64 / mtok * pricing.input
            + usage.output_tokens as f64 / mtok * pricing.output
            + usage.cache_creation_tokens as f64 / mtok * pricing.cache_write_rate()
            + usage.cache_read_tokens as f64 / mtok * pricing.cache_read_rate();
        Some(cost)
    }
}

fn default_override_path() -> PathBuf {
    // CLI 慣習に合わせ ~/.config を優先し、無ければ OS 標準の config dir
    // (macOS: ~/Library/Application Support) にフォールバック。
    if let Some(home) = dirs::home_dir() {
        let xdg = home.join(".config").join("llmeter").join("pricing.toml");
        if xdg.exists() {
            return xdg;
        }
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("llmeter")
        .join("pricing.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_with(
        user_entries: Vec<(String, ModelPricing)>,
        litellm: HashMap<String, ModelPricing>,
    ) -> PricingTable {
        PricingTable { user_entries, litellm, embedded_entries: embedded_defaults() }
    }

    fn embedded_only() -> PricingTable {
        table_with(vec![], HashMap::new())
    }

    #[test]
    fn known_model_matches_by_substring() {
        let table = embedded_only();
        let usage = Usage { input_tokens: 1_000_000, output_tokens: 1_000_000, ..Default::default() };
        let cost = table.calculate("claude-sonnet-5-20260115", &usage).unwrap();
        assert!((cost - 18.0).abs() < 1e-9);
    }

    #[test]
    fn unknown_model_returns_none() {
        let table = embedded_only();
        let usage = Usage::default();
        assert!(table.calculate("some-future-model", &usage).is_none());
    }

    #[test]
    fn cache_rates_default_relative_to_input() {
        let table = embedded_only();
        let usage = Usage {
            cache_creation_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
            ..Default::default()
        };
        // claude-sonnet: input 3.0 -> cache_write 3.75, cache_read 0.3
        let cost = table.calculate("claude-sonnet-5", &usage).unwrap();
        assert!((cost - 4.05).abs() < 1e-9);
    }

    fn pricing(input: f64, output: f64) -> ModelPricing {
        ModelPricing { input, output, cache_write: None, cache_read: None }
    }

    #[test]
    fn user_override_takes_priority_over_litellm() {
        let mut litellm = HashMap::new();
        litellm.insert("claude-sonnet-5".into(), pricing(999.0, 999.0));
        let table = table_with(vec![("claude-sonnet".into(), pricing(1.0, 2.0))], litellm);
        let (source, p) = table.resolve("claude-sonnet-5-20260115").unwrap();
        assert_eq!(source, PricingSource::User);
        assert_eq!(p.input, 1.0);
    }

    #[test]
    fn litellm_exact_match() {
        let mut litellm = HashMap::new();
        litellm.insert("claude-sonnet-5".into(), pricing(3.0, 15.0));
        let table = table_with(vec![], litellm);
        let (source, p) = table.resolve("claude-sonnet-5").unwrap();
        assert_eq!(source, PricingSource::LiteLlm);
        assert_eq!(p.input, 3.0);
    }

    #[test]
    fn litellm_prefix_stripped_exact_match() {
        let mut litellm = HashMap::new();
        litellm.insert("openai/gpt-4o".into(), pricing(2.5, 10.0));
        let table = table_with(vec![], litellm);
        let (source, p) = table.resolve("gpt-4o").unwrap();
        assert_eq!(source, PricingSource::LiteLlm);
        assert_eq!(p.input, 2.5);
    }

    #[test]
    fn litellm_longest_prefix_match() {
        let mut litellm = HashMap::new();
        litellm.insert("claude-sonnet-5".into(), pricing(3.0, 15.0));
        litellm.insert("claude-sonnet".into(), pricing(1.0, 1.0));
        let table = table_with(vec![], litellm);
        // "claude-sonnet-5-20260115" は "claude-sonnet-5" (15文字) の方が
        // "claude-sonnet" (13文字) より長く一致するのでそちらを採用
        let (source, p) = table.resolve("claude-sonnet-5-20260115").unwrap();
        assert_eq!(source, PricingSource::LiteLlm);
        assert_eq!(p.input, 3.0);
    }

    #[test]
    fn litellm_prefix_match_shorter_than_8_chars_is_rejected() {
        let mut litellm = HashMap::new();
        litellm.insert("zz-abc".into(), pricing(5.0, 15.0)); // 6文字 < 8、embedded にも非該当
        let table = table_with(vec![], litellm);
        assert!(table.resolve("zz-abc-nightly").is_none());
    }

    #[test]
    fn falls_back_to_embedded_when_litellm_has_no_match() {
        let table = table_with(vec![], HashMap::new());
        let (source, _) = table.resolve("claude-opus-4").unwrap();
        assert_eq!(source, PricingSource::Embedded);
    }
}
