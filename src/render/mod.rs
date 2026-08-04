pub mod html;
pub mod markdown;

use crate::aggregate::ModelStat;
use crate::model::{ModelUsage, Session, Tool};
use chrono::{DateTime, FixedOffset, Utc};

/// レポート表示では出さないモデル名（内部用・推定用）。
pub fn is_hidden_model_name(model: &str) -> bool {
    matches!(model, "cursor-unknown" | "<synthetic>" | "synthetic")
}

/// セッション詳細などのモデル一覧表示。
pub fn format_display_models(models: &[ModelUsage]) -> String {
    let names: Vec<&str> = models
        .iter()
        .map(|m| m.model.as_str())
        .filter(|m| !is_hidden_model_name(m))
        .collect();
    if names.is_empty() {
        "-".to_string()
    } else {
        names.join(", ")
    }
}

/// トランスクリプトの assistant 行モデルラベル。
pub fn format_assistant_model_label(model: Option<&str>) -> Option<&str> {
    model.filter(|m| !is_hidden_model_name(m))
}

pub fn visible_model_stats(by_model: &[ModelStat]) -> Vec<&ModelStat> {
    by_model
        .iter()
        .filter(|m| !is_hidden_model_name(&m.model))
        .collect()
}

fn jst_offset() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).expect("JST offset")
}

pub fn sort_sessions(sessions: &mut [Session], sort: &str) {
    match sort {
        "turns" => sessions.sort_by(|a, b| b.turns.cmp(&a.turns)),
        "errors" => sessions.sort_by(|a, b| {
            b.tool_error_rate()
                .partial_cmp(&a.tool_error_rate())
                .unwrap()
        }),
        "start" => sessions.sort_by(|a, b| b.start.cmp(&a.start)),
        "duration" => sessions.sort_by(|a, b| b.duration_secs().cmp(&a.duration_secs())),
        "prompt" => sessions.sort_by(|a, b| a.list_prompt().cmp(&b.list_prompt())),
        "repo" => sessions.sort_by(|a, b| {
            a.repo
                .as_deref()
                .unwrap_or("")
                .cmp(b.repo.as_deref().unwrap_or(""))
        }),
        "tool" => sessions.sort_by(|a, b| tool_sort_key(a.tool).cmp(&tool_sort_key(b.tool))),
        _ => sessions.sort_by(|a, b| b.cost.amount_usd.partial_cmp(&a.cost.amount_usd).unwrap()),
    }
}

fn tool_sort_key(tool: Tool) -> &'static str {
    tool.as_str()
}

/// 一覧の開始日時表示（JST）。
pub fn format_session_start(dt: DateTime<Utc>) -> String {
    dt.with_timezone(&jst_offset())
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

pub fn print_sessions_table(sessions: &[Session]) {
    if sessions.is_empty() {
        println!("セッションなし");
        return;
    }

    println!(
        "{:<40} {:>16} {:<10} {:<16} {:>5} {:>7} {:>8} {:>9}",
        "初回プロンプト",
        "開始(JST)",
        "ツール",
        "リポジトリ",
        "ターン",
        "エラー率",
        "所要時間",
        "コスト"
    );
    for s in sessions {
        let prompt = s.list_prompt().chars().take(40).collect::<String>();
        let start = format_session_start(s.start);
        let repo = s.repo.as_deref().unwrap_or("-");
        let duration = format_duration(s.duration_secs());
        let cost = if s.cost.has_unknown {
            format!("${:.2}+?", s.cost.amount_usd)
        } else {
            format!("${:.2}", s.cost.amount_usd)
        };
        println!(
            "{:<40} {:>16} {:<10} {:<16} {:>5} {:>6.0}% {:>8} {:>9}",
            prompt,
            start,
            s.tool.as_str(),
            repo,
            s.turns,
            s.tool_error_rate() * 100.0,
            duration,
            cost
        );
    }
}

pub fn format_duration(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h}h{m}m")
    } else {
        format!("{m}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn format_session_start_uses_jst() {
        let utc = Utc.with_ymd_and_hms(2026, 8, 4, 7, 0, 0).unwrap();
        assert_eq!(format_session_start(utc), "2026-08-04 16:00");
    }

    #[test]
    fn format_display_models_hides_internal_names() {
        let models = [
            ModelUsage {
                model: "claude-opus-4-6".into(),
                usage: crate::model::Usage::default(),
            },
            ModelUsage {
                model: "<synthetic>".into(),
                usage: crate::model::Usage::default(),
            },
            ModelUsage {
                model: "cursor-unknown".into(),
                usage: crate::model::Usage::default(),
            },
        ];
        assert_eq!(format_display_models(&models), "claude-opus-4-6");
        assert!(is_hidden_model_name("cursor-unknown"));
        assert!(is_hidden_model_name("<synthetic>"));
    }
}
