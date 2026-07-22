mod aggregate;
mod cache;
mod insights;
mod model;
mod pricing;
mod render;
mod sources;

use anyhow::Result;
use chrono::{Duration, Utc};
use clap::{Parser, Subcommand};
use model::{Session, Tool};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "llmeter", about = "AI コーディングツール利用分析 CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// レポート生成(HTML/Markdown)
    Report {
        #[arg(long, default_value_t = 30)]
        days: i64,
        #[arg(long, default_value = "html")]
        format: String,
        #[arg(long, default_value = "./llmeter-report/")]
        out: PathBuf,
        #[arg(long)]
        tools: Option<String>,
    },
    /// セッション一覧をターミナル表示
    Sessions {
        #[arg(long, default_value_t = 30)]
        days: i64,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long, default_value = "cost")]
        sort: String,
        #[arg(long)]
        tools: Option<String>,
    },
    /// セッション詳細トランスクリプト
    Session {
        id: String,
        #[arg(long, default_value = "md")]
        format: String,
    },
    /// キャッシュ操作
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    Clear,
    Status,
}

fn parse_tools(spec: &Option<String>) -> Vec<Tool> {
    match spec {
        None => vec![],
        Some(s) => s.split(',').filter_map(|t| Tool::parse(t.trim())).collect(),
    }
}

fn all_sources() -> Vec<Box<dyn sources::Source>> {
    vec![
        Box::new(sources::claude_code::ClaudeCodeSource::default()),
        Box::new(sources::codex::CodexSource::default()),
        Box::new(sources::cursor::CursorSource::default()),
    ]
}

fn collect_sessions(days: i64, tools: &[Tool]) -> Result<Vec<Session>> {
    let since = Utc::now() - Duration::days(days);
    let cache = cache::Cache::open()?;

    let mut result = Vec::new();
    for source in all_sources() {
        if !tools.is_empty() && !tools.contains(&source.tool()) {
            continue;
        }
        let files = source.discover().unwrap_or_default();
        for path in files {
            let sessions = match cache.get(&path)? {
                Some(cached) => cached,
                None => {
                    let parsed = source.parse_file(&path).unwrap_or_default();
                    cache.put(&path, parsed.clone()).ok();
                    parsed
                }
            };
            result.extend(sessions);
        }
    }

    Ok(aggregate::filter_since(result, Some(since)))
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Report { days, format, out, tools } => {
            let tools = parse_tools(&tools);
            let pricing = pricing::PricingTable::load(None);
            let mut sessions = collect_sessions(days, &tools)?;
            apply_cost(&mut sessions, &pricing);

            let overview = aggregate::build_overview(&sessions);
            let insight_lines = insights::generate(&sessions, Utc::now());

            std::fs::create_dir_all(&out)?;
            let is_md = matches!(format.as_str(), "md" | "markdown");
            if is_md {
                render::markdown::write_index(&out, &sessions, &overview, &insight_lines)?;
            } else {
                render::html::write_index(&out, &sessions, &overview, &insight_lines)?;
            }

            let sources = all_sources();
            for s in &sessions {
                let source = sources.iter().find(|src| src.tool() == s.tool);
                let Some(source) = source else { continue };
                let path = std::path::Path::new(&s.source_path);
                let transcript = match source.parse_transcript(path, &s.id) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if is_md {
                    render::markdown::write_session_detail(&out, &transcript)?;
                } else {
                    render::html::write_session_detail(&out, &transcript)?;
                }
            }

            println!("レポート出力: {}", out.display());
        }
        Command::Sessions { days, repo, sort, tools } => {
            let tools = parse_tools(&tools);
            let pricing = pricing::PricingTable::load(None);
            let mut sessions = collect_sessions(days, &tools)?;
            apply_cost(&mut sessions, &pricing);

            if let Some(repo) = &repo {
                sessions.retain(|s| s.repo.as_deref() == Some(repo.as_str()));
            }
            sort_sessions(&mut sessions, &sort);
            render::print_sessions_table(&sessions);
        }
        Command::Session { id, format } => {
            print_session_detail(&id, &format)?;
        }
        Command::Cache { action } => {
            let cache = cache::Cache::open()?;
            match action {
                CacheAction::Clear => {
                    cache.clear()?;
                    println!("キャッシュをクリアした");
                }
                CacheAction::Status => {
                    let (count, size) = cache.status()?;
                    println!("キャッシュ件数: {count}, サイズ: {} KB", size / 1024);
                }
            }
        }
    }

    Ok(())
}

fn apply_cost(sessions: &mut [Session], pricing: &pricing::PricingTable) {
    for s in sessions.iter_mut() {
        let mut total = 0.0;
        let mut has_unknown = false;
        for mu in &s.models {
            match pricing.calculate(&mu.model, &mu.usage) {
                Some(c) => total += c,
                None => has_unknown = true,
            }
        }
        s.cost = model::Cost { amount_usd: total, has_unknown };
    }
}

fn sort_sessions(sessions: &mut [Session], sort: &str) {
    match sort {
        "turns" => sessions.sort_by(|a, b| b.turns.cmp(&a.turns)),
        "errors" => sessions.sort_by(|a, b| {
            b.tool_error_rate()
                .partial_cmp(&a.tool_error_rate())
                .unwrap()
        }),
        _ => sessions.sort_by(|a, b| b.cost.amount_usd.partial_cmp(&a.cost.amount_usd).unwrap()),
    }
}

fn print_session_detail(id: &str, format: &str) -> Result<()> {
    let cache = cache::Cache::open()?;

    for source in all_sources() {
        for path in source.discover().unwrap_or_default() {
            let matches = match cache.get(&path)? {
                Some(cached) => cached.iter().any(|s| s.id == id),
                None => false,
            };
            let matches = if matches {
                true
            } else {
                // キャッシュ未存在 or 別セッション → 軽量メタだけ確認するため一旦パース
                let parsed = source.parse_file(&path).unwrap_or_default();
                let hit = parsed.iter().any(|s| s.id == id);
                cache.put(&path, parsed).ok();
                hit
            };

            if matches {
                let mut transcript = source.parse_transcript(&path, id)?;
                let pricing = pricing::PricingTable::load(None);
                apply_cost(std::slice::from_mut(&mut transcript.session), &pricing);
                match format {
                    "html" => render::html::print_session_detail(&transcript),
                    _ => render::markdown::print_session_detail(&transcript),
                }
                return Ok(());
            }
        }
    }

    println!("セッションが見つからない: {id}");
    Ok(())
}
