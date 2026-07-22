use crate::pricing::ModelPricing;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const SOURCE_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const TTL_SECS: u64 = 7 * 24 * 3600;
const TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Deserialize, Default)]
struct RawEntry {
    #[serde(default)]
    input_cost_per_token: Option<f64>,
    #[serde(default)]
    output_cost_per_token: Option<f64>,
    #[serde(default)]
    cache_creation_input_token_cost: Option<f64>,
    #[serde(default)]
    cache_read_input_token_cost: Option<f64>,
}

/// LiteLLM の生 JSON をパースし、モデルキー（小文字）→ 単価 の Map を返す。
/// input/output どちらか欠けたエントリと `sample_spec` はスキップする。
pub fn parse(raw: &str) -> HashMap<String, ModelPricing> {
    let map: HashMap<String, RawEntry> = match serde_json::from_str(raw) {
        Ok(m) => m,
        Err(_) => return HashMap::new(),
    };
    let mtok = 1_000_000.0;
    map.into_iter()
        .filter(|(key, _)| key != "sample_spec")
        .filter_map(|(key, entry)| {
            let input = entry.input_cost_per_token?;
            let output = entry.output_cost_per_token?;
            let pricing = ModelPricing {
                input: input * mtok,
                output: output * mtok,
                cache_write: entry.cache_creation_input_token_cost.map(|c| c * mtok),
                cache_read: entry.cache_read_input_token_cost.map(|c| c * mtok),
            };
            Some((key.to_lowercase(), pricing))
        })
        .collect()
}

pub fn cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("llmeter")
        .join("litellm_prices.json")
}

fn is_fresh(path: &std::path::Path) -> bool {
    let Ok(meta) = fs::metadata(path) else { return false };
    let Ok(mtime) = meta.modified() else { return false };
    SystemTime::now()
        .duration_since(mtime)
        .map(|age| age.as_secs() < TTL_SECS)
        .unwrap_or(false)
}

fn fetch_remote() -> Result<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .build();
    let body = agent
        .get(SOURCE_URL)
        .call()
        .context("LiteLLM 料金データの取得に失敗した")?
        .into_string()
        .context("LiteLLM 料金データの読み取りに失敗した")?;
    Ok(body)
}

fn write_cache(path: &std::path::Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, body)?;
    Ok(())
}

/// 3層構造の第2層（LiteLLM）を読み込む。
/// `offline` の場合はネットワークアクセスせずキャッシュのみ使用する。
/// キャッシュも取得も失敗した場合は None（呼び出し側で embedded_defaults にフォールバック）。
pub fn load(offline: bool) -> Option<HashMap<String, ModelPricing>> {
    let path = cache_path();

    if !offline && !is_fresh(&path) {
        match fetch_remote() {
            Ok(body) => {
                if let Err(e) = write_cache(&path, &body) {
                    eprintln!("警告: LiteLLM 料金データのキャッシュ書き込みに失敗した: {e}");
                }
                return Some(parse(&body));
            }
            Err(e) => {
                if path.exists() {
                    eprintln!("警告: LiteLLM 料金データの取得に失敗、古いキャッシュを使用する: {e}");
                } else {
                    eprintln!(
                        "警告: LiteLLM 料金データの取得に失敗、キャッシュもないため埋め込みデフォルトを使用する: {e}"
                    );
                }
            }
        }
    }

    let raw = fs::read_to_string(&path).ok()?;
    Some(parse(&raw))
}

/// TTL を無視して強制的に再取得し、キャッシュを更新する。パースできたモデル数を返す。
pub fn refresh() -> Result<usize> {
    let body = fetch_remote()?;
    let path = cache_path();
    write_cache(&path, &body)?;
    Ok(parse(&body).len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "sample_spec": {
            "input_cost_per_token": 0.000001,
            "output_cost_per_token": 0.000002
        },
        "claude-sonnet-5": {
            "input_cost_per_token": 0.000003,
            "output_cost_per_token": 0.000015,
            "cache_creation_input_token_cost": 0.00000375,
            "cache_read_input_token_cost": 0.0000003
        },
        "openai/gpt-4o": {
            "input_cost_per_token": 0.0000025,
            "output_cost_per_token": 0.00001,
            "cache_creation_input_token_cost": null,
            "cache_read_input_token_cost": null
        },
        "missing-output": {
            "input_cost_per_token": 0.000001
        }
    }"#;

    #[test]
    fn sample_spec_is_excluded() {
        let parsed = parse(FIXTURE);
        assert!(!parsed.contains_key("sample_spec"));
    }

    #[test]
    fn entries_missing_input_or_output_are_skipped() {
        let parsed = parse(FIXTURE);
        assert!(!parsed.contains_key("missing-output"));
    }

    #[test]
    fn converts_per_token_to_per_million_tokens() {
        let parsed = parse(FIXTURE);
        let p = parsed.get("claude-sonnet-5").unwrap();
        assert!((p.input - 3.0).abs() < 1e-9);
        assert!((p.output - 15.0).abs() < 1e-9);
        assert!((p.cache_write.unwrap() - 3.75).abs() < 1e-9);
        assert!((p.cache_read.unwrap() - 0.3).abs() < 1e-9);
    }

    #[test]
    fn null_cache_costs_are_none() {
        let parsed = parse(FIXTURE);
        let p = parsed.get("openai/gpt-4o").unwrap();
        assert!(p.cache_write.is_none());
        assert!(p.cache_read.is_none());
    }

    #[test]
    fn keys_are_lowercased() {
        let parsed = parse(FIXTURE);
        assert!(parsed.contains_key("openai/gpt-4o"));
    }
}
