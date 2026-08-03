use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Default)]
struct ConfigFile {
    #[serde(default)]
    report: ReportConfig,
}

/// `config.toml` の `[report]` セクション。
#[derive(Debug, Clone, Deserialize)]
pub struct ReportConfig {
    #[serde(default = "default_days")]
    pub days: i64,
    pub since: Option<NaiveDate>,
    pub until: Option<NaiveDate>,
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_out")]
    pub out: PathBuf,
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub offline: bool,
    pub analyze: Option<String>,
    #[serde(default = "default_analyze_timeout")]
    pub analyze_timeout: u64,
    #[serde(default)]
    pub stdout: bool,
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ReportConfig {
    pub fn builtin() -> Self {
        Self {
            days: default_days(),
            since: None,
            until: None,
            format: default_format(),
            out: default_out(),
            tools: None,
            offline: false,
            analyze: None,
            analyze_timeout: default_analyze_timeout(),
            stdout: false,
        }
    }
}

/// `llmeter report` 実行時に使う、CLI と config をマージした値。
#[derive(Debug, Clone)]
pub struct ResolvedReportOptions {
    pub days: i64,
    pub since: Option<NaiveDate>,
    pub until: Option<NaiveDate>,
    pub format: String,
    pub out: PathBuf,
    pub tools: Option<String>,
    pub offline: bool,
    pub analyze: Option<String>,
    pub analyze_timeout: u64,
    pub stdout: bool,
}

#[derive(Debug, Default)]
pub struct ReportCliOptions {
    pub days: Option<i64>,
    pub since: Option<NaiveDate>,
    pub until: Option<NaiveDate>,
    pub format: Option<String>,
    pub out: Option<PathBuf>,
    pub tools: Option<String>,
    pub offline: bool,
    pub analyze: Option<String>,
    pub analyze_timeout: Option<u64>,
    pub stdout: bool,
}

pub fn load_report_config() -> ReportConfig {
    match load_config_file() {
        Some(file) => file.report,
        None => ReportConfig::builtin(),
    }
}

pub fn resolve_report_options(cli: ReportCliOptions) -> Result<ResolvedReportOptions> {
    let defaults = load_report_config();
    let format = cli.format.unwrap_or(defaults.format);
    if format != "html" && format != "md" {
        bail!("format は html または md を指定してください: {format}");
    }
    if let Some(agent) = cli.analyze.as_deref().or(defaults.analyze.as_deref()) {
        match agent {
            "claude" | "codex" | "cursor" => {}
            other => bail!("analyze は claude / codex / cursor を指定してください: {other}"),
        }
    }
    let stdout = if cli.stdout { true } else { defaults.stdout };
    if stdout && format != "md" {
        bail!("--stdout は --format md と併用する必要がある");
    }
    let tools = cli
        .tools
        .or_else(|| defaults.tools.map(|items| items.join(",")));
    Ok(ResolvedReportOptions {
        days: cli.days.unwrap_or(defaults.days),
        since: cli.since.or(defaults.since),
        until: cli.until.or(defaults.until),
        format,
        out: expand_home(cli.out.unwrap_or(defaults.out)),
        tools,
        offline: if cli.offline { true } else { defaults.offline },
        analyze: cli.analyze.or(defaults.analyze),
        analyze_timeout: cli.analyze_timeout.unwrap_or(defaults.analyze_timeout),
        stdout,
    })
}

fn load_config_file() -> Option<ConfigFile> {
    let path = config_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content)
        .with_context(|| format!("設定ファイルの解析に失敗: {}", path.display()))
        .ok()
}

pub fn config_path() -> Option<PathBuf> {
    config_path_candidates().into_iter().find(|p| p.exists())
}

fn config_path_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".config").join("llmeter").join("config.toml"));
    }
    if let Some(dir) = dirs::config_dir() {
        let candidate = dir.join("llmeter").join("config.toml");
        if !paths.contains(&candidate) {
            paths.push(candidate);
        }
    }
    paths
}

pub fn expand_home(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if s == "~" {
        return dirs::home_dir().unwrap_or(path);
    }
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    path
}

fn default_days() -> i64 {
    30
}

fn default_format() -> String {
    "html".into()
}

fn default_out() -> PathBuf {
    PathBuf::from("./llmeter-report/")
}

fn default_analyze_timeout() -> u64 {
    300
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn expands_tilde_in_path() {
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(
            expand_home(PathBuf::from("~/reports/llmeter")),
            home.join("reports/llmeter")
        );
        assert_eq!(expand_home(PathBuf::from("~")), home);
    }

    #[test]
    fn builtin_defaults_match_current_cli() {
        let d = ReportConfig::builtin();
        assert_eq!(d.days, 30);
        assert_eq!(d.format, "html");
        assert_eq!(d.out, PathBuf::from("./llmeter-report/"));
        assert!(!d.offline);
        assert!(!d.stdout);
    }

    #[test]
    fn cli_overrides_config_values() {
        let defaults = ReportConfig {
            days: 7,
            format: "md".into(),
            out: PathBuf::from("./from-config"),
            tools: Some(vec!["claude".into()]),
            offline: true,
            analyze: Some("codex".into()),
            analyze_timeout: 600,
            stdout: false,
            ..ReportConfig::builtin()
        };
        let cli = ReportCliOptions {
            days: Some(90),
            format: Some("html".into()),
            out: Some(PathBuf::from("./cli-out")),
            tools: None,
            offline: false,
            analyze: None,
            analyze_timeout: None,
            stdout: false,
            since: None,
            until: None,
        };
        let resolved = merge_for_test(&cli, &defaults);
        assert_eq!(resolved.days, 90);
        assert_eq!(resolved.format, "html");
        assert_eq!(resolved.out, PathBuf::from("./cli-out"));
        assert_eq!(resolved.tools.as_deref(), Some("claude"));
        assert!(resolved.offline);
        assert_eq!(resolved.analyze.as_deref(), Some("codex"));
        assert_eq!(resolved.analyze_timeout, 600);

        let cli = ReportCliOptions { offline: true, ..Default::default() };
        let defaults = ReportConfig { offline: false, ..ReportConfig::builtin() };
        let resolved = merge_for_test(&cli, &defaults);
        assert!(resolved.offline);
    }

    #[test]
    fn parses_config_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let mut file = std::fs::File::create(&path).unwrap();
        write!(
            file,
            r#"
[report]
days = 14
format = "md"
out = "~/reports/llmeter"
tools = ["claude", "cursor"]
offline = true
analyze = "claude"
analyze_timeout = 120
stdout = false
since = "2026-06-01"
"#
        )
        .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: ConfigFile = toml::from_str(&content).unwrap();
        assert_eq!(parsed.report.days, 14);
        assert_eq!(parsed.report.format, "md");
        assert_eq!(parsed.report.out, PathBuf::from("~/reports/llmeter"));
        assert_eq!(
            parsed.report.tools,
            Some(vec!["claude".into(), "cursor".into()])
        );
        assert!(parsed.report.offline);
        assert_eq!(parsed.report.analyze.as_deref(), Some("claude"));
        assert_eq!(parsed.report.analyze_timeout, 120);
        assert_eq!(
            parsed.report.since,
            NaiveDate::from_ymd_opt(2026, 6, 1)
        );
    }

    fn merge_for_test(cli: &ReportCliOptions, defaults: &ReportConfig) -> ResolvedReportOptions {
        let format = cli.format.clone().unwrap_or_else(|| defaults.format.clone());
        let tools = cli
            .tools
            .clone()
            .or_else(|| defaults.tools.clone().map(|items| items.join(",")));
        ResolvedReportOptions {
            days: cli.days.unwrap_or(defaults.days),
            since: cli.since.or(defaults.since),
            until: cli.until.or(defaults.until),
            format,
            out: expand_home(cli.out.clone().unwrap_or_else(|| defaults.out.clone())),
            tools,
            offline: if cli.offline { true } else { defaults.offline },
            analyze: cli.analyze.clone().or_else(|| defaults.analyze.clone()),
            analyze_timeout: cli.analyze_timeout.unwrap_or(defaults.analyze_timeout),
            stdout: if cli.stdout { true } else { defaults.stdout },
        }
    }
}
