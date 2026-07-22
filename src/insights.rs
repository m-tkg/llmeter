use crate::model::Session;
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;

/// ルールベースの「気づき」を生成する。sessions は表示期間分を渡す想定。
pub fn generate(sessions: &[Session], now: DateTime<Utc>) -> Vec<String> {
    let mut out = Vec::new();

    let week_ago = now - Duration::days(7);
    let two_weeks_ago = now - Duration::days(14);

    let recent: Vec<&Session> = sessions.iter().filter(|s| s.start >= week_ago).collect();
    let prev: Vec<&Session> = sessions
        .iter()
        .filter(|s| s.start >= two_weeks_ago && s.start < week_ago)
        .collect();

    // 直近7日コスト最大リポジトリと前週比
    if let Some((repo, cost)) = top_repo_cost(&recent) {
        let prev_cost = repo_cost(&prev, &repo);
        let msg = if prev_cost > 0.0 {
            let diff_pct = (cost - prev_cost) / prev_cost * 100.0;
            format!(
                "直近7日で最もコストがかかったリポジトリは `{repo}`（${cost:.2}、前週比 {diff_pct:+.0}%）"
            )
        } else {
            format!("直近7日で最もコストがかかったリポジトリは `{repo}`（${cost:.2}）")
        };
        out.push(msg);
    }

    // ツール別コスト比率
    if let Some(msg) = tool_cost_ratio_insight(&recent) {
        out.push(msg);
    }

    // 直近7日のセッション数
    out.push(format!("直近7日のセッション数: {}件", recent.len()));

    // ツールエラー率が高いセッション傾向
    if let Some(msg) = high_error_rate_insight(&recent) {
        out.push(msg);
    }

    out
}

fn top_repo_cost(sessions: &[&Session]) -> Option<(String, f64)> {
    let mut map: BTreeMap<String, f64> = BTreeMap::new();
    for s in sessions {
        if let Some(repo) = &s.repo {
            *map.entry(repo.clone()).or_insert(0.0) += s.cost.amount_usd;
        }
    }
    map.into_iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
}

fn repo_cost(sessions: &[&Session], repo: &str) -> f64 {
    sessions
        .iter()
        .filter(|s| s.repo.as_deref() == Some(repo))
        .map(|s| s.cost.amount_usd)
        .sum()
}

fn tool_cost_ratio_insight(sessions: &[&Session]) -> Option<String> {
    let total: f64 = sessions.iter().map(|s| s.cost.amount_usd).sum();
    if total <= 0.0 {
        return None;
    }
    let mut map: BTreeMap<&'static str, f64> = BTreeMap::new();
    for s in sessions {
        *map.entry(s.tool.as_str()).or_insert(0.0) += s.cost.amount_usd;
    }
    let (tool, cost) = map.into_iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())?;
    let ratio = cost / total * 100.0;
    Some(format!("直近7日のコストは {tool} が全体の{ratio:.0}%を占める"))
}

fn high_error_rate_insight(sessions: &[&Session]) -> Option<String> {
    let threshold = 0.2;
    let high: Vec<&&Session> = sessions.iter().filter(|s| s.tool_error_rate() > threshold).collect();
    if high.is_empty() {
        return None;
    }
    Some(format!(
        "直近7日でツールエラー率が{:.0}%を超えるセッションが{}件あり",
        threshold * 100.0,
        high.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cost, Tool, ToolCallStat, Usage};
    use chrono::TimeZone;

    fn session(days_ago: i64, repo: &str, cost: f64, error_rate_pair: (u64, u64)) -> Session {
        let now = Utc.with_ymd_and_hms(2026, 7, 22, 0, 0, 0).unwrap();
        Session {
            tool: Tool::ClaudeCode,
            id: "id".into(),
            source_path: "p".into(),
            cwd: None,
            repo: Some(repo.into()),
            start: now - Duration::days(days_ago),
            end: now - Duration::days(days_ago) + Duration::hours(1),
            turns: 1,
            first_prompt: None,
            models: vec![],
            usage: Usage::default(),
            tool_calls: vec![ToolCallStat {
                name: "Bash".into(),
                count: error_rate_pair.0,
                error_count: error_rate_pair.1,
            }],
            cost: Cost { amount_usd: cost, has_unknown: false },
        }
    }

    #[test]
    fn generates_expected_insight_count() {
        let now = Utc.with_ymd_and_hms(2026, 7, 22, 0, 0, 0).unwrap();
        let sessions = vec![
            session(1, "repoA", 5.0, (10, 3)),
            session(10, "repoA", 2.0, (10, 0)),
        ];
        let insights = generate(&sessions, now);
        assert!(insights.iter().any(|i| i.contains("repoA")));
        assert!(insights.iter().any(|i| i.contains("セッション数")));
        assert!(insights.iter().any(|i| i.contains("エラー率")));
    }
}
