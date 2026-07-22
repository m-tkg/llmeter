pub mod html;
pub mod markdown;

use crate::model::Session;

pub fn print_sessions_table(sessions: &[Session]) {
    if sessions.is_empty() {
        println!("セッションなし");
        return;
    }

    println!(
        "{:<50} {:<10} {:<20} {:>6} {:>8} {:>10} {:>10}",
        "初回プロンプト", "ツール", "リポジトリ", "ターン", "エラー率", "所要時間", "コスト"
    );
    for s in sessions {
        let prompt = s
            .first_prompt
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(50)
            .collect::<String>();
        let repo = s.repo.as_deref().unwrap_or("-");
        let duration = format_duration(s.duration_secs());
        let cost = if s.cost.has_unknown {
            format!("${:.2}+?", s.cost.amount_usd)
        } else {
            format!("${:.2}", s.cost.amount_usd)
        };
        println!(
            "{:<50} {:<10} {:<20} {:>6} {:>7.0}% {:>10} {:>10}",
            prompt,
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
