pub mod html;
pub mod markdown;

use crate::model::{Session, Tool};
use chrono::{DateTime, Utc};

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

/// 一覧の開始日時表示（UTC）。
pub fn format_session_start(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M").to_string()
}

pub fn print_sessions_table(sessions: &[Session]) {
    if sessions.is_empty() {
        println!("セッションなし");
        return;
    }

    println!(
        "{:<40} {:>16} {:<10} {:<16} {:>5} {:>7} {:>8} {:>9}",
        "初回プロンプト",
        "開始(UTC)",
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
