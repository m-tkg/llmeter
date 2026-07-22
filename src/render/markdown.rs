use crate::aggregate::Overview;
use crate::model::{Session, Transcript, TranscriptEvent};
use anyhow::Result;
use std::fmt::Write as _;
use std::path::Path;

/// レポート本文（Overview〜セッション一覧）の Markdown を組み立てる。
/// `--format md` の report.md 本体と、`--analyze` 時の分析入力の両方で使う。
/// `analysis` を渡すと「内訳（リポジトリ別）」の直後に AI 分析セクションをマージする。
pub fn build_index_markdown(
    sessions: &[Session],
    overview: &Overview,
    insight_lines: &[String],
    analysis: Option<(&str, &str)>,
) -> Result<String> {
    let mut md = String::new();
    writeln!(md, "# llmeter レポート\n")?;

    writeln!(md, "## Overview\n")?;
    let cost_str = if overview.has_unknown_cost {
        format!("${:.2}+ (未知モデル分含まず)", overview.total_cost)
    } else {
        format!("${:.2}", overview.total_cost)
    };
    writeln!(md, "- 期間コスト: {cost_str}")?;
    writeln!(md, "- 総トークン: {}", overview.total_tokens)?;
    writeln!(md, "- セッション数: {}", overview.session_count)?;
    writeln!(md, "- アクティブ時間: {}", super::format_duration(overview.active_seconds))?;
    writeln!(md, "- ターン数中央値: {:.1}", overview.median_turns)?;
    writeln!(md, "- 平均ツールエラー率: {:.1}%\n", overview.mean_tool_error_rate * 100.0)?;

    writeln!(md, "### 今週の気づき\n")?;
    for line in insight_lines {
        writeln!(md, "- {line}")?;
    }
    writeln!(md)?;

    writeln!(md, "### 日別コスト\n")?;
    let max_daily = overview.daily.iter().map(|d| d.total_cost).fold(0.0_f64, f64::max);
    for day in &overview.daily {
        let bar = bar_chart(day.total_cost, max_daily, 30);
        writeln!(md, "- {} {} ${:.2}", day.date, bar, day.total_cost)?;
    }
    writeln!(md)?;

    writeln!(md, "### ツール別コスト\n")?;
    let max_tool = overview.by_tool.iter().map(|t| t.cost).fold(0.0_f64, f64::max);
    for t in &overview.by_tool {
        let bar = bar_chart(t.cost, max_tool, 30);
        writeln!(md, "- {:<12} {} ${:.2} ({} セッション)", t.tool, bar, t.cost, t.sessions)?;
    }
    writeln!(md)?;

    writeln!(md, "### モデル別\n")?;
    for m in &overview.by_model {
        let unknown = if m.has_unknown { " (未知単価あり)" } else { "" };
        writeln!(md, "- {}: ${:.2}, {} tokens{}", m.model, m.cost, m.tokens, unknown)?;
    }
    writeln!(md)?;

    writeln!(md, "### リポジトリ別\n")?;
    for r in &overview.by_repo {
        writeln!(md, "- {}: ${:.2} ({} セッション)", r.repo, r.cost, r.sessions)?;
    }
    writeln!(md)?;

    if let Some((agent, content)) = analysis {
        writeln!(md, "## AI 分析（{agent}）\n")?;
        writeln!(md, "{content}\n")?;
    }

    writeln!(md, "## セッション一覧\n")?;
    writeln!(md, "| 初回プロンプト | ツール | リポジトリ | ターン | エラー率 | 所要時間 | コスト |")?;
    writeln!(md, "|---|---|---|---|---|---|---|")?;
    for s in sessions {
        let prompt = truncate(s.first_prompt.as_deref().unwrap_or(""), 50);
        let repo = s.repo.as_deref().unwrap_or("-");
        let cost = if s.cost.has_unknown {
            format!("${:.2}+?", s.cost.amount_usd)
        } else {
            format!("${:.2}", s.cost.amount_usd)
        };
        writeln!(
            md,
            "| [{}](sessions/{}.md) | {} | {} | {} | {:.0}% | {} | {} |",
            prompt,
            s.id,
            s.tool.as_str(),
            repo,
            s.turns,
            s.tool_error_rate() * 100.0,
            super::format_duration(s.duration_secs()),
            cost
        )?;
    }

    Ok(md)
}

pub fn write_index(
    out_dir: &Path,
    sessions: &[Session],
    overview: &Overview,
    insight_lines: &[String],
    analysis: Option<(&str, &str)>,
) -> Result<()> {
    let md = build_index_markdown(sessions, overview, insight_lines, analysis)?;
    std::fs::write(out_dir.join("report.md"), md)?;
    Ok(())
}

pub fn write_session_detail(out_dir: &Path, transcript: &Transcript) -> Result<()> {
    let md = render_session_markdown(transcript);
    let sessions_dir = out_dir.join("sessions");
    std::fs::create_dir_all(&sessions_dir)?;
    std::fs::write(sessions_dir.join(format!("{}.md", transcript.session.id)), md)?;
    Ok(())
}

pub fn print_session_detail(transcript: &Transcript) {
    println!("{}", render_session_markdown(transcript));
}

fn render_session_markdown(t: &Transcript) -> String {
    let s = &t.session;
    let mut md = String::new();
    let _ = writeln!(md, "# セッション詳細: {}\n", s.id);
    let _ = writeln!(md, "- リポジトリ: {}", s.repo.as_deref().unwrap_or("-"));
    let _ = writeln!(
        md,
        "- モデル: {}",
        s.models.iter().map(|m| m.model.as_str()).collect::<Vec<_>>().join(", ")
    );
    let _ = writeln!(md, "- 期間: {} 〜 {}", s.start, s.end);
    let cost = if s.cost.has_unknown {
        format!("${:.2}+?", s.cost.amount_usd)
    } else {
        format!("${:.2}", s.cost.amount_usd)
    };
    let _ = writeln!(md, "- コスト: {cost}");
    let _ = writeln!(md, "- トークン: {}\n", s.usage.total());

    let _ = writeln!(md, "## トランスクリプト\n");
    for ev in &t.events {
        match ev {
            TranscriptEvent::UserMessage { timestamp, text } => {
                let _ = writeln!(md, "**User** ({timestamp}):\n\n{text}\n");
            }
            TranscriptEvent::AssistantMessage { timestamp, text, model } => {
                let model_label = model.as_deref().unwrap_or("");
                let _ = writeln!(md, "**Assistant** [{model_label}] ({timestamp}):\n\n{text}\n");
            }
            TranscriptEvent::ToolUse { timestamp, name, summary, is_error } => {
                let mark = if *is_error { "✗" } else { "▶" };
                let _ = writeln!(md, "{mark} {name} ({summary}) — {timestamp}\n");
            }
            TranscriptEvent::Marker { timestamp, label } => {
                let _ = writeln!(md, "— {label} ({timestamp}) —\n");
            }
        }
    }

    md
}

fn bar_chart(value: f64, max: f64, width: usize) -> String {
    if max <= 0.0 {
        return String::new();
    }
    let filled = ((value / max) * width as f64).round() as usize;
    "▇".repeat(filled.min(width))
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
