use crate::model::Session;
use chrono::{DateTime, NaiveDate, Utc};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct DailyStat {
    pub date: NaiveDate,
    pub cost_by_tool: BTreeMap<&'static str, f64>,
    pub total_cost: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ToolStat {
    pub tool: &'static str,
    pub cost: f64,
    pub sessions: u64,
    pub tokens: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ModelStat {
    pub model: String,
    pub cost: f64,
    pub tokens: u64,
    pub has_unknown: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RepoStat {
    pub repo: String,
    pub cost: f64,
    pub sessions: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Overview {
    pub total_cost: f64,
    pub has_unknown_cost: bool,
    pub total_tokens: u64,
    pub session_count: u64,
    pub active_seconds: i64,
    pub daily: Vec<DailyStat>,
    pub by_tool: Vec<ToolStat>,
    pub by_model: Vec<ModelStat>,
    pub by_repo: Vec<RepoStat>,
    pub median_turns: f64,
    pub mean_tool_error_rate: f64,
}

pub fn build_overview(sessions: &[Session]) -> Overview {
    let mut overview = Overview::default();
    let mut daily_map: BTreeMap<NaiveDate, DailyStat> = BTreeMap::new();
    let mut tool_map: BTreeMap<&'static str, ToolStat> = BTreeMap::new();
    let mut model_map: BTreeMap<String, ModelStat> = BTreeMap::new();
    let mut repo_map: BTreeMap<String, RepoStat> = BTreeMap::new();
    let mut turns_list: Vec<u32> = Vec::new();
    let mut error_rates: Vec<f64> = Vec::new();

    for s in sessions {
        overview.total_cost += s.cost.amount_usd;
        overview.has_unknown_cost = overview.has_unknown_cost || s.cost.has_unknown;
        overview.total_tokens += s.usage.total();
        overview.session_count += 1;
        overview.active_seconds += s.duration_secs();
        turns_list.push(s.turns);
        error_rates.push(s.tool_error_rate());

        let date = s.start.date_naive();
        let day = daily_map.entry(date).or_insert_with(|| DailyStat {
            date,
            ..Default::default()
        });
        *day.cost_by_tool.entry(s.tool.as_str()).or_insert(0.0) += s.cost.amount_usd;
        day.total_cost += s.cost.amount_usd;

        let tool_stat = tool_map.entry(s.tool.as_str()).or_insert_with(|| ToolStat {
            tool: s.tool.as_str(),
            ..Default::default()
        });
        tool_stat.cost += s.cost.amount_usd;
        tool_stat.sessions += 1;
        tool_stat.tokens += s.usage.total();

        for mu in &s.models {
            let entry = model_map.entry(mu.model.clone()).or_insert_with(|| ModelStat {
                model: mu.model.clone(),
                ..Default::default()
            });
            entry.tokens += mu.usage.total();
        }

        if let Some(repo) = &s.repo {
            let entry = repo_map.entry(repo.clone()).or_insert_with(|| RepoStat {
                repo: repo.clone(),
                ..Default::default()
            });
            entry.cost += s.cost.amount_usd;
            entry.sessions += 1;
        }
    }

    // モデル別コストはセッション単位の cost しか持たないため、
    // モデルが単一のセッションはそのままコストを積む。複数モデル混在セッションは
    // トークン比率で按分する。
    for s in sessions {
        if s.models.len() == 1 {
            if let Some(entry) = model_map.get_mut(&s.models[0].model) {
                entry.cost += s.cost.amount_usd;
                entry.has_unknown = entry.has_unknown || s.cost.has_unknown;
            }
        } else if !s.models.is_empty() {
            let total_tok: u64 = s.models.iter().map(|m| m.usage.total()).sum();
            for mu in &s.models {
                if let Some(entry) = model_map.get_mut(&mu.model) {
                    let ratio = if total_tok > 0 {
                        mu.usage.total() as f64 / total_tok as f64
                    } else {
                        0.0
                    };
                    entry.cost += s.cost.amount_usd * ratio;
                    entry.has_unknown = entry.has_unknown || s.cost.has_unknown;
                }
            }
        }
    }

    overview.daily = daily_map.into_values().collect();
    overview.by_tool = tool_map.into_values().collect();
    overview.by_tool.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap());
    overview.by_model = model_map.into_values().collect();
    overview.by_model.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap());
    overview.by_repo = repo_map.into_values().collect();
    overview.by_repo.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap());

    overview.median_turns = median_u32(&mut turns_list);
    overview.mean_tool_error_rate = if error_rates.is_empty() {
        0.0
    } else {
        error_rates.iter().sum::<f64>() / error_rates.len() as f64
    };

    overview
}

fn median_u32(values: &mut [u32]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] as f64 + values[mid] as f64) / 2.0
    } else {
        values[mid] as f64
    }
}

pub fn filter_since(sessions: Vec<Session>, since: Option<DateTime<Utc>>) -> Vec<Session> {
    match since {
        Some(s) => sessions.into_iter().filter(|sess| sess.start >= s).collect(),
        None => sessions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cost, ModelUsage, Tool, ToolCallStat, Usage};
    use chrono::TimeZone;

    fn session(tool: Tool, cost: f64, model: &str, tokens: u64, turns: u32) -> Session {
        Session {
            tool,
            id: "id".into(),
            source_path: "p".into(),
            cwd: Some("/repo".into()),
            repo: Some("repo".into()),
            start: Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 7, 1, 1, 0, 0).unwrap(),
            turns,
            first_prompt: None,
            models: vec![ModelUsage {
                model: model.into(),
                usage: Usage { input_tokens: tokens, ..Default::default() },
            }],
            usage: Usage { input_tokens: tokens, ..Default::default() },
            tool_calls: vec![ToolCallStat { name: "Bash".into(), count: 2, error_count: 1 }],
            cost: Cost { amount_usd: cost, has_unknown: false },
        }
    }

    #[test]
    fn overview_aggregates_cost_and_tokens() {
        let sessions = vec![
            session(Tool::ClaudeCode, 1.0, "claude-sonnet-5", 100, 3),
            session(Tool::Codex, 2.0, "gpt-5", 200, 5),
        ];
        let ov = build_overview(&sessions);
        assert!((ov.total_cost - 3.0).abs() < 1e-9);
        assert_eq!(ov.total_tokens, 300);
        assert_eq!(ov.session_count, 2);
        assert_eq!(ov.by_tool.len(), 2);
        assert_eq!(ov.median_turns, 4.0);
        assert!((ov.mean_tool_error_rate - 0.5).abs() < 1e-9);
    }
}
