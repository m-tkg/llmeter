mod aggregate;
mod analyze;
mod cache;
mod insights;
mod litellm;
mod model;
mod pricing;
mod render;
mod sources;

use anyhow::Result;
use chrono::{Duration, Utc};
use clap::{Parser, Subcommand};
use model::{Session, Tool};
use std::path::PathBuf;

const REPO_URL: &str = "https://github.com/m-tkg/llmeter";

#[derive(Parser)]
#[command(
    name = "llmeter",
    version,
    about = "AI コーディングツール（Claude Code / Codex / Cursor）の利用状況をローカルログから集計・可視化する CLI",
    after_help = "\
例:
  llmeter report                          直近30日を HTML で ./llmeter-report/ に出力
  llmeter report --format md              同じ内容を Markdown で出力
  llmeter report --days 7 --out ./weekly  直近7日を ./weekly/ に出力
  llmeter report --since 2026-06-01 --until 2026-06-30 --format md
                                           2026年6月分を Markdown で出力
  llmeter report --format md --stdout | pbcopy
                                           Markdown をファイルに書かず標準出力へ
  llmeter sessions --sort errors          ツールエラー率順にセッション一覧を表示
  llmeter session <ID>                    セッション詳細を Markdown で標準出力
  llmeter report --analyze claude         AI にコスト削減提案を分析させレポートにマージ

詳細は各サブコマンドの --help（例: llmeter report --help）を参照。"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// レポート生成。HTML（ダッシュボード）または Markdown を --out ディレクトリに書き出す
    #[command(after_help = "\
出力構成（HTML/Markdown 共通）:
  <out>/index.html   (または report.md)   Overview + セッション一覧
  <out>/sessions/<ID>.html (または .md)   各セッションの詳細（一覧からリンク）

例:
  llmeter report                                # HTML、直近30日、./llmeter-report/
  llmeter report --format md                    # Markdown で ./llmeter-report/ に出力
  llmeter report --format md --out ./docs/usage # Markdown を任意のディレクトリへ
  llmeter report --days 90 --tools claude,codex # 期間90日、対象ツールを限定
  llmeter report --since 2026-06-01 --until 2026-06-30 --format md # 日付範囲指定
  llmeter report --format md --stdout           # Markdown を標準出力へ（ファイル書き込みなし）
  llmeter report --analyze claude               # レポートを claude に読ませ AI 分析をマージ")]
    Report {
        /// 集計対象期間（日数）。--since/--until 指定時は無視される
        #[arg(long, default_value_t = 30)]
        days: i64,
        /// 集計開始日（YYYY-MM-DD、この日を含む）。指定時は --days を無視
        #[arg(long, value_name = "DATE")]
        since: Option<chrono::NaiveDate>,
        /// 集計終了日（YYYY-MM-DD、この日を含む）。指定時は --days を無視
        #[arg(long, value_name = "DATE")]
        until: Option<chrono::NaiveDate>,
        /// 出力形式
        #[arg(long, default_value = "html", value_parser = ["html", "md"])]
        format: String,
        /// 出力先ディレクトリ（なければ作成）
        #[arg(long, default_value = "./llmeter-report/")]
        out: PathBuf,
        /// 対象ツールをカンマ区切りで限定（claude, codex, cursor）。省略時は全ツール
        #[arg(long, value_name = "TOOLS")]
        tools: Option<String>,
        /// ネットワークアクセスなしで実行（LiteLLM 料金データはキャッシュ+埋め込みのみ使用）
        #[arg(long)]
        offline: bool,
        /// レポートを AI エージェント CLI に読ませ、コスト削減提案をマージする（要インストール・認証済み）
        #[arg(long, value_parser = ["claude", "codex", "cursor"])]
        analyze: Option<String>,
        /// --analyze 実行時のタイムアウト（秒）
        #[arg(long, default_value_t = 300)]
        analyze_timeout: u64,
        /// ファイルに書き出さず Markdown を標準出力に出す（--format md 専用）
        #[arg(long)]
        stdout: bool,
    },
    /// セッション一覧をターミナルにテーブル表示
    #[command(after_help = "\
例:
  llmeter sessions                     # 直近30日、コスト降順
  llmeter sessions --repo llmeter      # リポジトリ名で絞り込み
  llmeter sessions --sort turns        # ターン数順")]
    Sessions {
        /// 集計対象期間（日数）
        #[arg(long, default_value_t = 30)]
        days: i64,
        /// リポジトリ名で絞り込み（部分一致）
        #[arg(long)]
        repo: Option<String>,
        /// 並び順
        #[arg(long, default_value = "cost", value_parser = ["cost", "turns", "errors"])]
        sort: String,
        /// 対象ツールをカンマ区切りで限定（claude, codex, cursor）。省略時は全ツール
        #[arg(long, value_name = "TOOLS")]
        tools: Option<String>,
        /// ネットワークアクセスなしで実行（LiteLLM 料金データはキャッシュ+埋め込みのみ使用）
        #[arg(long)]
        offline: bool,
    },
    /// セッション詳細トランスクリプトを標準出力に表示
    #[command(after_help = "\
セッション ID は `llmeter sessions` や HTML レポートのリンク先ファイル名で確認できる。

例:
  llmeter session 0151c9d7-7a81-4429-a1d1-e1b4d878a64e                # Markdown
  llmeter session 0151c9d7-... --format html > session.html          # HTML を保存")]
    Session {
        /// セッション ID（`llmeter sessions` で確認）
        id: String,
        /// 出力形式
        #[arg(long, default_value = "md", value_parser = ["md", "html"])]
        format: String,
        /// ネットワークアクセスなしで実行（LiteLLM 料金データはキャッシュ+埋め込みのみ使用）
        #[arg(long)]
        offline: bool,
    },
    /// 増分キャッシュの操作（macOS: ~/Library/Caches/llmeter, Linux: ~/.cache/llmeter）
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// LiteLLM 料金データベースの操作
    Pricing {
        #[command(subcommand)]
        action: PricingAction,
    },
    /// llmeter 自身を最新版に更新する（cargo install --git で再ビルド）
    #[command(after_help = "\
GitHub の main ブランチの版と比較し、新しければ
`cargo install --git https://github.com/m-tkg/llmeter --force` を実行する。
ビルドに1〜2分かかる。cargo が必要。")]
    Update {
        /// バージョンが同じでも強制的に再インストールする
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum PricingAction {
    /// TTL を無視して LiteLLM 料金データを強制的に再取得する
    Refresh,
    /// 指定モデルの解決結果（採用層・単価）を表示する
    Show {
        /// モデル名（ログ上の表記、例: claude-sonnet-5-20260115）
        model: String,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    /// キャッシュを削除する（次回実行時に全ログを再パース）
    Clear,
    /// キャッシュの件数とサイズを表示する
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

fn resolve_date_range(
    days: i64,
    since: Option<chrono::NaiveDate>,
    until: Option<chrono::NaiveDate>,
) -> (Option<chrono::DateTime<Utc>>, Option<chrono::DateTime<Utc>>) {
    if since.is_none() && until.is_none() {
        return (Some(Utc::now() - Duration::days(days)), None);
    }
    let since_dt = since.and_then(|d| d.and_hms_opt(0, 0, 0)).map(|dt| dt.and_utc());
    let until_dt = until.and_then(|d| d.and_hms_opt(23, 59, 59)).map(|dt| dt.and_utc());
    (since_dt, until_dt)
}

fn collect_sessions(
    since: Option<chrono::DateTime<Utc>>,
    until: Option<chrono::DateTime<Utc>>,
    tools: &[Tool],
) -> Result<Vec<Session>> {
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

    Ok(aggregate::filter_range(result, since, until))
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Report {
            days,
            since,
            until,
            format,
            out,
            tools,
            offline,
            analyze,
            analyze_timeout,
            stdout,
        } => {
            if stdout && format != "md" {
                anyhow::bail!("--stdout は --format md と併用する必要がある");
            }
            let tools = parse_tools(&tools);
            let pricing = pricing::PricingTable::load(None, offline);
            let (since_dt, until_dt) = resolve_date_range(days, since, until);
            let mut sessions = collect_sessions(since_dt, until_dt, &tools)?;
            apply_cost(&mut sessions, &pricing);

            let overview = aggregate::build_overview(&sessions);
            let insight_lines = insights::generate(&sessions, Utc::now());

            let analysis_text = if let Some(agent) = &analyze {
                if !stdout {
                    println!("AI 分析中 ({agent})...");
                }
                let input_md = render::markdown::build_index_markdown(&sessions, &overview, &insight_lines, None)?;
                analyze::run_agent(agent, &input_md, analyze_timeout)
            } else {
                None
            };
            let analysis = analyze.as_deref().zip(analysis_text.as_deref());

            if stdout {
                let md = render::markdown::build_index_markdown(&sessions, &overview, &insight_lines, analysis)?;
                print!("{md}");
                return Ok(());
            }

            std::fs::create_dir_all(&out)?;
            let is_md = matches!(format.as_str(), "md" | "markdown");
            if is_md {
                render::markdown::write_index(&out, &sessions, &overview, &insight_lines, analysis)?;
            } else {
                render::html::write_index(&out, &sessions, &overview, &insight_lines, analysis)?;
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
        Command::Sessions { days, repo, sort, tools, offline } => {
            let tools = parse_tools(&tools);
            let pricing = pricing::PricingTable::load(None, offline);
            let (since_dt, until_dt) = resolve_date_range(days, None, None);
            let mut sessions = collect_sessions(since_dt, until_dt, &tools)?;
            apply_cost(&mut sessions, &pricing);

            if let Some(repo) = &repo {
                sessions.retain(|s| s.repo.as_deref() == Some(repo.as_str()));
            }
            sort_sessions(&mut sessions, &sort);
            render::print_sessions_table(&sessions);
        }
        Command::Session { id, format, offline } => {
            print_session_detail(&id, &format, offline)?;
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
        Command::Pricing { action } => match action {
            PricingAction::Refresh => match litellm::refresh() {
                Ok(count) => println!("LiteLLM 料金データを更新した（{count} モデル）"),
                Err(e) => eprintln!("LiteLLM 料金データの更新に失敗した: {e}"),
            },
            PricingAction::Show { model } => {
                let pricing = pricing::PricingTable::load(None, false);
                match pricing.resolve(&model) {
                    Some((source, p)) => {
                        println!("モデル: {model}");
                        println!("採用層: {}", source.as_str());
                        println!("input:       {:.4} $/1M tokens", p.input);
                        println!("output:      {:.4} $/1M tokens", p.output);
                        println!("cache_write: {:.4} $/1M tokens", p.cache_write_rate());
                        println!("cache_read:  {:.4} $/1M tokens", p.cache_read_rate());
                    }
                    None => println!("未知モデル: {model}（料金データが見つからない）"),
                }
            }
        },
        Command::Update { force } => self_update(force)?,
    }

    Ok(())
}

/// リモート main の Cargo.toml から version を取得する（失敗時 None）。
fn fetch_remote_version() -> Option<String> {
    let url = "https://raw.githubusercontent.com/m-tkg/llmeter/main/Cargo.toml";
    let body = ureq::get(url)
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let parsed: toml::Value = toml::from_str(&body).ok()?;
    Some(parsed.get("package")?.get("version")?.as_str()?.to_string())
}

fn self_update(force: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("現在のバージョン: {current}");

    match fetch_remote_version() {
        Some(remote) if remote == current && !force => {
            println!("最新版です（{remote}）。--force で強制再インストールできる。");
            return Ok(());
        }
        Some(remote) => println!("リモートのバージョン: {remote}。更新する。"),
        None => println!("リモートのバージョン確認に失敗。そのまま再インストールする。"),
    }

    let status = std::process::Command::new("cargo")
        .args(["install", "--git", REPO_URL, "--force"])
        .status();
    match status {
        Ok(s) if s.success() => println!("更新完了。"),
        Ok(s) => anyhow::bail!("cargo install が失敗した (exit: {s})"),
        Err(e) => anyhow::bail!("cargo を実行できない: {e}（cargo のインストールが必要）"),
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

fn print_session_detail(id: &str, format: &str, offline: bool) -> Result<()> {
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
                let pricing = pricing::PricingTable::load(None, offline);
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
