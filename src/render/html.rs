use crate::aggregate::{DailyStat, ModelStat, Overview, RepoStat, ToolStat};
use crate::model::{Session, Transcript, TranscriptEvent};
use anyhow::Result;
use std::fmt::Write as _;
use std::path::Path;

const TOOL_ORDER: [&str; 3] = ["claude-code", "codex", "cursor"];

const STYLE: &str = r#"
:root {
  --page: #0d0d0d;
  --surface: #1a1a19;
  --border: rgba(255,255,255,0.10);
  --ink: #ffffff;
  --ink-2: #c3c2b7;
  --muted: #898781;
  --grid: #2c2c2a;
  --baseline: #383835;
  --claude: #3987e5;
  --codex: #d95926;
  --cursor: #199e70;
  --accent: #3987e5;
  --good: #0ca30c;
  --bad: #d03b3b;
}
* { box-sizing: border-box; }
body {
  margin: 0;
  background: var(--page);
  color: var(--ink);
  font-family: system-ui, -apple-system, "Segoe UI", sans-serif;
}
.page { max-width: 1080px; margin: 0 auto; padding: 32px; }
.header-row { display: flex; align-items: baseline; justify-content: space-between; margin-bottom: 16px; }
.brand { font-size: 16px; font-weight: 600; }
.period { font-size: 13px; color: var(--muted); }
.card { background: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 20px 24px; margin-bottom: 16px; }
.stat-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 16px; margin-bottom: 16px; }
@media (max-width: 640px) { .stat-grid { grid-template-columns: repeat(2, 1fr); } }
.stat-card { background: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 20px 24px; }
.stat-label { font-size: 12px; color: var(--muted); }
.stat-value { font-size: 28px; font-weight: 650; margin: 4px 0; font-variant-numeric: tabular-nums; }
.stat-note { font-size: 12px; color: var(--muted); }
.insights h3 { font-size: 14px; font-weight: 600; margin: 0 0 12px; }
.insights .bolt { color: var(--accent); }
.insights ul { margin: 0 0 12px; padding-left: 18px; color: var(--ink-2); font-size: 13.5px; line-height: 1.9; }
.insights ul:last-child { margin-bottom: 0; }
.insights p { margin: 0 0 12px; color: var(--ink-2); font-size: 13.5px; line-height: 1.9; }
.insights p:last-child { margin-bottom: 0; }
.insights code { background: var(--grid); padding: 1px 5px; border-radius: 4px; font-size: 0.9em; }
.section-title { font-size: 14px; font-weight: 600; margin: 0 0 12px; color: var(--ink); }
.chart-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
.legend { display: flex; gap: 16px; font-size: 12px; color: var(--ink-2); }
.swatch { display: inline-block; width: 10px; height: 10px; border-radius: 3px; margin-right: 6px; vertical-align: middle; }
.daily-chart { width: 100%; height: auto; display: block; }
.hit-rect { fill: transparent; cursor: pointer; }
.bar-list .row { display: flex; align-items: center; gap: 12px; margin: 10px 0; }
.bar-list .name { width: 120px; flex-shrink: 0; color: var(--ink-2); font-size: 13px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.bar-list .track { flex: 1; background: var(--grid); height: 12px; border-radius: 6px; overflow: hidden; }
.bar-list .fill { height: 100%; border-radius: 0 4px 4px 0; }
.bar-list .val { color: var(--ink); font-variant-numeric: tabular-nums; font-size: 13px; white-space: nowrap; }
.breakdown-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
@media (max-width: 640px) { .breakdown-grid { grid-template-columns: 1fr; } }
.breakdown-label { font-size: 13px; font-weight: 600; color: var(--ink-2); margin: 0 0 8px; }
.mini-stats { display: flex; gap: 24px; margin-bottom: 16px; font-size: 13px; color: var(--ink-2); }
.mini-stats b { color: var(--ink); font-variant-numeric: tabular-nums; font-weight: 650; }
table { width: 100%; border-collapse: collapse; }
th { text-align: left; font-size: 11px; color: var(--muted); font-weight: 600; padding: 8px 10px; border-bottom: 1px solid var(--border); }
td { padding: 10px; border-bottom: 1px solid var(--grid); font-size: 13.5px; color: var(--ink-2); }
tbody tr:hover td { background: rgba(255,255,255,0.03); }
td.num { text-align: right; font-variant-numeric: tabular-nums; }
td.cost { font-weight: 600; color: var(--ink); }
.tool-cell { white-space: nowrap; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
.error-dot { display: inline-block; width: 6px; height: 6px; border-radius: 50%; background: var(--bad); margin-left: 4px; }
.tooltip { position: absolute; display: none; background: var(--surface); border: 1px solid var(--border); border-radius: 8px; padding: 8px 12px; font-size: 12px; color: var(--ink-2); pointer-events: none; z-index: 10; line-height: 1.6; }
.back-link { font-size: 13px; color: var(--accent); }
.meta-sub { font-size: 12px; color: var(--muted); margin: 4px 0 16px; }
.transcript .event { margin: 16px 0; }
.role { font-size: 11px; color: var(--muted); font-weight: 600; letter-spacing: .02em; margin-bottom: 4px; }
.event-body { color: var(--ink-2); white-space: pre-wrap; font-size: 13.5px; line-height: 1.6; }
details.tool-call { margin: 12px 0; }
details.tool-call summary { font-size: 13px; color: var(--muted); cursor: pointer; list-style: none; }
details.tool-call summary::-webkit-details-marker { display: none; }
.marker-row { text-align: center; color: var(--muted); font-size: 11px; margin: 16px 0; }
"#;

const TOOLTIP_JS: &str = r#"
(function () {
  var tip = document.getElementById('chart-tooltip');
  if (!tip) return;
  document.querySelectorAll('.hit-rect').forEach(function (el) {
    el.addEventListener('mouseenter', function () {
      tip.innerHTML = el.getAttribute('data-tooltip') || '';
      tip.style.display = 'block';
      var bar = document.getElementById(el.getAttribute('data-bar'));
      if (bar) bar.style.filter = 'brightness(1.15)';
    });
    el.addEventListener('mousemove', function (e) {
      tip.style.left = (e.pageX + 12) + 'px';
      tip.style.top = (e.pageY + 12) + 'px';
    });
    el.addEventListener('mouseleave', function () {
      tip.style.display = 'none';
      var bar = document.getElementById(el.getAttribute('data-bar'));
      if (bar) bar.style.filter = '';
    });
  });
})();
"#;

pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn tool_color(tool: &str) -> &'static str {
    match tool {
        "claude-code" => "var(--claude)",
        "codex" => "var(--codex)",
        "cursor" => "var(--cursor)",
        _ => "var(--muted)",
    }
}

fn tool_label(tool: &str) -> &'static str {
    match tool {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "cursor" => "Cursor",
        _ => "Unknown",
    }
}

fn format_tokens_short(n: u64) -> String {
    let f = n as f64;
    if f >= 1_000_000.0 {
        format!("{:.1}M", f / 1_000_000.0)
    } else if f >= 1_000.0 {
        format!("{:.1}K", f / 1_000.0)
    } else {
        n.to_string()
    }
}

fn nice_number(rough: f64) -> f64 {
    if rough <= 0.0 {
        return 1.0;
    }
    let exp = rough.log10().floor();
    let magnitude = 10f64.powf(exp);
    let residual = rough / magnitude;
    let nice = if residual <= 1.0 {
        1.0
    } else if residual <= 2.0 {
        2.0
    } else if residual <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * magnitude
}

/// Y軸目盛を「きれいな数字」3〜4本で生成する（0始まり、最後の値が軸最大値）。
fn axis_ticks(max_value: f64) -> Vec<f64> {
    if max_value <= 0.0 {
        return vec![0.0, 1.0];
    }
    let step = nice_number(max_value / 3.0);
    let axis_max = (max_value / step).ceil() * step;
    let count = (axis_max / step).round().max(1.0) as usize;
    (0..=count).map(|i| step * i as f64).collect()
}

fn page(title: &str, body: &str, include_tooltip_js: bool) -> String {
    let script = if include_tooltip_js {
        format!("<script>{TOOLTIP_JS}</script>")
    } else {
        String::new()
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title><style>{}</style></head><body><div class=\"page\">{}</div>{}</body></html>",
        html_escape(title),
        STYLE,
        body,
        script
    )
}

/// 「前週比 +25%）」のような箇所を検出し、▲▼ + 色付き数値に装飾する。それ以外はエスケープするのみ。
fn render_insight_line(line: &str) -> String {
    const MARK: &str = "前週比 ";
    if let Some(pos) = line.find(MARK) {
        let after_start = pos + MARK.len();
        let after = &line[after_start..];
        if let Some(pct_pos) = after.find('%') {
            let num_str = &after[..pct_pos];
            let mut chars = num_str.chars();
            if let Some(sign) = chars.next() {
                let rest_ok = num_str.len() > 1 && chars.clone().all(|c| c.is_ascii_digit() || c == '.');
                if (sign == '+' || sign == '-') && rest_ok {
                    let before = &line[..after_start];
                    let after_pct = &after[pct_pos + 1..];
                    let (color, arrow) = if sign == '+' { ("var(--bad)", "▲") } else { ("var(--good)", "▼") };
                    let num_display = &num_str[1..];
                    return format!(
                        "{}<span style=\"color:{color}\">{arrow} {sign}{num_display}%</span>{}",
                        html_escape(before),
                        html_escape(after_pct)
                    );
                }
            }
        }
    }
    html_escape(line)
}

fn render_stat_card(label: &str, value_html: &str, note: &str) -> String {
    format!(
        "<div class=\"stat-card\"><div class=\"stat-label\">{}</div><div class=\"stat-value\">{}</div><div class=\"stat-note\">{}</div></div>",
        html_escape(label),
        value_html,
        html_escape(note)
    )
}

fn render_legend() -> String {
    let mut s = String::from("<div class=\"legend\">");
    for tool in TOOL_ORDER {
        let _ = write!(
            s,
            "<span><span class=\"swatch\" style=\"background:{}\"></span>{}</span>",
            tool_color(tool),
            tool_label(tool)
        );
    }
    s.push_str("</div>");
    s
}

/// 棒セグメントのSVG path。round_top時のみ上端2角を半径radiusで丸める。
fn segment_path(x: f64, y_top: f64, y_bottom: f64, w: f64, radius: f64, round_top: bool) -> String {
    let h = y_bottom - y_top;
    if h <= 0.0 {
        return String::new();
    }
    let x2 = x + w;
    if !round_top || radius <= 0.0 {
        return format!("M{x:.1} {y_bottom:.1} L{x:.1} {y_top:.1} L{x2:.1} {y_top:.1} L{x2:.1} {y_bottom:.1} Z");
    }
    let r = radius.min(w / 2.0).min(h);
    format!(
        "M{x:.1} {y_bottom:.1} L{x:.1} {:.1} A{r:.1} {r:.1} 0 0 1 {:.1} {y_top:.1} L{:.1} {y_top:.1} A{r:.1} {r:.1} 0 0 1 {x2:.1} {:.1} L{x2:.1} {y_bottom:.1} Z",
        y_top + r,
        x + r,
        x2 - r,
        y_top + r
    )
}

fn render_daily_chart(daily: &[DailyStat]) -> String {
    let w = 1000.0_f64;
    let h = 260.0_f64;
    let margin_left = 44.0;
    let margin_right = 12.0;
    let margin_top = 16.0;
    let margin_bottom = 28.0;
    let chart_w = w - margin_left - margin_right;
    let chart_h = h - margin_top - margin_bottom;

    let max_daily = daily.iter().map(|d| d.total_cost).fold(0.0_f64, f64::max);
    let ticks = axis_ticks(max_daily);
    let axis_max = ticks.last().copied().unwrap_or(1.0).max(0.000_001);
    let y_scale = chart_h / axis_max;

    let n = daily.len().max(1);
    let slot_w = chart_w / n as f64;
    let bar_w = (slot_w * 0.6).clamp(2.0, 24.0);

    let mut svg = String::new();
    let _ = write!(svg, "<svg viewBox=\"0 0 {w} {h}\" class=\"daily-chart\" role=\"img\" aria-label=\"日別コスト\">");

    for t in &ticks {
        let y = margin_top + chart_h - t * y_scale;
        let color = if *t == 0.0 { "var(--baseline)" } else { "var(--grid)" };
        let _ = write!(
            svg,
            "<line x1=\"{margin_left:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\" stroke=\"{color}\" stroke-width=\"1\"/>",
            w - margin_right
        );
        let _ = write!(
            svg,
            "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"11\" fill=\"var(--muted)\" text-anchor=\"end\">${:.0}</text>",
            margin_left - 8.0,
            y + 3.5,
            t
        );
    }

    let label_step = ((n as f64) / 6.0).ceil().max(1.0) as usize;
    for (i, day) in daily.iter().enumerate() {
        if i % label_step != 0 {
            continue;
        }
        let cx = margin_left + slot_w * i as f64 + slot_w / 2.0;
        let _ = write!(
            svg,
            "<text x=\"{cx:.1}\" y=\"{:.1}\" font-size=\"11\" fill=\"var(--muted)\" text-anchor=\"middle\">{}</text>",
            h - 8.0,
            day.date.format("%m/%d")
        );
    }

    for (i, day) in daily.iter().enumerate() {
        let cx = margin_left + slot_w * i as f64 + slot_w / 2.0;
        let x = cx - bar_w / 2.0;
        let baseline_y = margin_top + chart_h;

        let active: Vec<(&str, f64)> = TOOL_ORDER
            .iter()
            .filter_map(|t| {
                let v = day.cost_by_tool.get(t).copied().unwrap_or(0.0);
                if v > 0.0 {
                    Some((*t, v))
                } else {
                    None
                }
            })
            .collect();

        let _ = write!(svg, "<g id=\"bar-{i}\">");
        let mut y_cursor = baseline_y;
        for (idx, (tool, val)) in active.iter().enumerate() {
            let full_h = val * y_scale;
            let seg_top = y_cursor - full_h;
            let seg_bottom = y_cursor - if idx > 0 { 2.0 } else { 0.0 };
            let is_topmost = idx == active.len() - 1;
            let path = segment_path(x, seg_top, seg_bottom, bar_w, 4.0, is_topmost);
            if !path.is_empty() {
                let _ = write!(svg, "<path d=\"{path}\" fill=\"{}\"/>", tool_color(tool));
            }
            y_cursor = seg_top;
        }
        svg.push_str("</g>");

        let mut tooltip = format!("<b>{}</b><br>", day.date);
        for (tool, val) in &active {
            let _ = write!(tooltip, "{}: ${val:.2}<br>", tool_label(tool));
        }
        let _ = write!(tooltip, "合計: ${:.2}", day.total_cost);

        let _ = write!(
            svg,
            "<rect class=\"hit-rect\" data-bar=\"bar-{i}\" data-tooltip=\"{}\" x=\"{:.1}\" y=\"{margin_top:.1}\" width=\"{slot_w:.1}\" height=\"{chart_h:.1}\"/>",
            html_escape(&tooltip),
            cx - slot_w / 2.0
        );
    }

    svg.push_str("</svg>");
    svg
}

fn render_tool_bar_chart(by_tool: &[ToolStat]) -> String {
    let max = by_tool.iter().map(|t| t.cost).fold(0.0_f64, f64::max);
    let mut s = String::from("<div class=\"bar-list\">");
    for t in by_tool {
        let pct = if max > 0.0 { (t.cost / max * 100.0).max(2.0) } else { 0.0 };
        let color = tool_color(t.tool);
        let _ = write!(
            s,
            "<div class=\"row\"><span class=\"name\"><span class=\"swatch\" style=\"background:{color}\"></span>{}</span><div class=\"track\"><div class=\"fill\" style=\"width:{pct:.1}%;background:{color}\"></div></div><span class=\"val\">${:.2} · {} sessions</span></div>",
            tool_label(t.tool),
            t.cost,
            t.sessions
        );
    }
    s.push_str("</div>");
    s
}

fn render_model_bar_list(by_model: &[ModelStat]) -> String {
    let max = by_model.iter().map(|m| m.cost).fold(0.0_f64, f64::max);
    let mut s = String::from("<div class=\"bar-list\">");
    for m in by_model.iter().take(10) {
        let pct = if max > 0.0 { (m.cost / max * 100.0).max(2.0) } else { 0.0 };
        let unknown = if m.has_unknown {
            " <span style=\"color:var(--muted)\">(単価未知)</span>"
        } else {
            ""
        };
        let _ = write!(
            s,
            "<div class=\"row\"><span class=\"name\">{}</span><div class=\"track\"><div class=\"fill\" style=\"width:{pct:.1}%;background:var(--accent)\"></div></div><span class=\"val\">${:.2}{unknown}</span></div>",
            html_escape(&m.model),
            m.cost
        );
    }
    if by_model.len() > 10 {
        let rest_cost: f64 = by_model[10..].iter().map(|m| m.cost).sum();
        let rest_n = by_model.len() - 10;
        let _ = write!(
            s,
            "<div class=\"row\"><span class=\"name\" style=\"color:var(--muted)\">他 {rest_n} 件</span><span class=\"val\" style=\"color:var(--muted)\">${rest_cost:.2}</span></div>"
        );
    }
    s.push_str("</div>");
    s
}

fn render_repo_bar_list(by_repo: &[RepoStat]) -> String {
    let max = by_repo.iter().map(|r| r.cost).fold(0.0_f64, f64::max);
    let mut s = String::from("<div class=\"bar-list\">");
    for r in by_repo.iter().take(10) {
        let pct = if max > 0.0 { (r.cost / max * 100.0).max(2.0) } else { 0.0 };
        let _ = write!(
            s,
            "<div class=\"row\"><span class=\"name\">{}</span><div class=\"track\"><div class=\"fill\" style=\"width:{pct:.1}%;background:var(--accent)\"></div></div><span class=\"val\">${:.2}</span></div>",
            html_escape(&r.repo),
            r.cost
        );
    }
    if by_repo.len() > 10 {
        let rest_cost: f64 = by_repo[10..].iter().map(|r| r.cost).sum();
        let rest_n = by_repo.len() - 10;
        let _ = write!(
            s,
            "<div class=\"row\"><span class=\"name\" style=\"color:var(--muted)\">他 {rest_n} 件</span><span class=\"val\" style=\"color:var(--muted)\">${rest_cost:.2}</span></div>"
        );
    }
    s.push_str("</div>");
    s
}

pub fn write_index(
    out_dir: &Path,
    sessions: &[Session],
    overview: &Overview,
    insight_lines: &[String],
    analysis: Option<(&str, &str)>,
) -> Result<()> {
    let mut body = String::new();

    let period_text = if let (Some(first), Some(last)) = (overview.daily.first(), overview.daily.last()) {
        let days = (last.date - first.date).num_days() + 1;
        format!("直近{days}日 · {} 〜 {}", first.date, last.date)
    } else {
        String::new()
    };
    let _ = write!(
        body,
        "<div class=\"header-row\"><span class=\"brand\">llmeter</span><span class=\"period\">{}</span></div>",
        html_escape(&period_text)
    );

    let cost_str = if overview.has_unknown_cost {
        format!("${:.2}+", overview.total_cost)
    } else {
        format!("${:.2}", overview.total_cost)
    };
    let cost_note = if overview.has_unknown_cost { "未知モデル分含まず" } else { "全ツール合算" };
    let _ = write!(
        body,
        "<div class=\"stat-grid\">{}{}{}{}</div>",
        render_stat_card("期間コスト", &html_escape(&cost_str), cost_note),
        render_stat_card("総トークン", &format_tokens_short(overview.total_tokens), "入出力合計"),
        render_stat_card("セッション数", &overview.session_count.to_string(), "全ツール"),
        render_stat_card("アクティブ時間", &super::format_duration(overview.active_seconds), "合計"),
    );

    let _ = write!(body, "<div class=\"card insights\"><h3><span class=\"bolt\">⚡</span> 今週の気づき</h3><ul>");
    for line in insight_lines {
        let _ = write!(body, "<li>{}</li>", render_insight_line(line));
    }
    body.push_str("</ul></div>");

    let _ = write!(
        body,
        "<div class=\"card\"><div class=\"chart-head\"><h2 class=\"section-title\">日別コスト</h2>{}</div>{}</div>",
        render_legend(),
        render_daily_chart(&overview.daily)
    );

    let _ = write!(
        body,
        "<div class=\"card\"><h2 class=\"section-title\">ツール別コスト</h2>{}</div>",
        render_tool_bar_chart(&overview.by_tool)
    );

    let _ = write!(
        body,
        "<div class=\"card\"><h2 class=\"section-title\">内訳</h2><div class=\"breakdown-grid\"><div><div class=\"breakdown-label\">モデル別</div>{}</div><div><div class=\"breakdown-label\">リポジトリ別</div>{}</div></div></div>",
        render_model_bar_list(&overview.by_model),
        render_repo_bar_list(&overview.by_repo)
    );

    if let Some((agent, content)) = analysis {
        let inner = crate::analyze::markdown_to_html(content);
        let _ = write!(
            body,
            "<div class=\"card insights\"><h3>🤖 AI 分析（{}）</h3>{}</div>",
            html_escape(agent),
            inner
        );
    }

    let _ = write!(
        body,
        "<div class=\"card\"><h2 class=\"section-title\">セッション一覧</h2><div class=\"mini-stats\"><span>ターン数中央値 <b>{:.1}</b></span><span>平均エラー率 <b>{:.1}%</b></span><span>セッション数 <b>{}</b></span></div>",
        overview.median_turns,
        overview.mean_tool_error_rate * 100.0,
        overview.session_count
    );
    body.push_str(
        "<table><thead><tr><th>初回プロンプト</th><th>ツール</th><th>リポジトリ</th><th class=\"num\">ターン</th><th class=\"num\">エラー率</th><th class=\"num\">所要時間</th><th class=\"num\">コスト</th></tr></thead><tbody>",
    );
    for s in sessions {
        let prompt = truncate(s.first_prompt.as_deref().unwrap_or(""), 50);
        let repo = s.repo.as_deref().unwrap_or("-");
        let cost = if s.cost.has_unknown {
            format!("${:.2}+?", s.cost.amount_usd)
        } else {
            format!("${:.2}", s.cost.amount_usd)
        };
        let err_pct = s.tool_error_rate() * 100.0;
        let err_html = if err_pct <= 0.0 {
            format!("<span style=\"color:var(--muted)\">{err_pct:.0}%</span>")
        } else if err_pct > 10.0 {
            format!("{err_pct:.0}%<span class=\"error-dot\"></span>")
        } else {
            format!("{err_pct:.0}%")
        };
        let _ = write!(
            body,
            "<tr><td><a href=\"sessions/{}.html\">{}</a></td><td class=\"tool-cell\"><span class=\"swatch\" style=\"background:{}\"></span>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num cost\">{}</td></tr>",
            s.id,
            html_escape(&prompt),
            tool_color(s.tool.as_str()),
            tool_label(s.tool.as_str()),
            html_escape(repo),
            s.turns,
            err_html,
            super::format_duration(s.duration_secs()),
            html_escape(&cost)
        );
    }
    body.push_str("</tbody></table></div>");
    body.push_str("<div id=\"chart-tooltip\" class=\"tooltip\"></div>");

    let html = page("llmeter レポート", &body, true);
    std::fs::write(out_dir.join("index.html"), html)?;
    Ok(())
}

pub fn write_session_detail(out_dir: &Path, transcript: &Transcript) -> Result<()> {
    let html = render_session_html(transcript);
    let sessions_dir = out_dir.join("sessions");
    std::fs::create_dir_all(&sessions_dir)?;
    std::fs::write(sessions_dir.join(format!("{}.html", transcript.session.id)), html)?;
    Ok(())
}

pub fn print_session_detail(transcript: &Transcript) {
    println!("{}", render_session_html(transcript));
}

fn render_session_html(t: &Transcript) -> String {
    let s = &t.session;
    let mut body = String::new();

    body.push_str("<div class=\"header-row\"><a class=\"back-link\" href=\"../index.html\">← レポートに戻る</a></div>");

    let cost_str = if s.cost.has_unknown {
        format!("${:.2}+?", s.cost.amount_usd)
    } else {
        format!("${:.2}", s.cost.amount_usd)
    };
    let _ = write!(
        body,
        "<div class=\"stat-grid\">{}{}{}{}</div>",
        render_stat_card("コスト", &html_escape(&cost_str), ""),
        render_stat_card("トークン", &format_tokens_short(s.usage.total()), ""),
        render_stat_card("ターン", &s.turns.to_string(), ""),
        render_stat_card("所要時間", &super::format_duration(s.duration_secs()), ""),
    );

    let models: Vec<&str> = s.models.iter().map(|m| m.model.as_str()).collect();
    let _ = write!(
        body,
        "<div class=\"meta-sub\">リポジトリ: {} · モデル: {} · 期間: {} 〜 {}</div>",
        html_escape(s.repo.as_deref().unwrap_or("-")),
        html_escape(&models.join(", ")),
        s.start,
        s.end
    );

    body.push_str("<div class=\"card transcript\"><h2 class=\"section-title\">トランスクリプト</h2>");
    for ev in &t.events {
        match ev {
            TranscriptEvent::UserMessage { timestamp, text } => {
                let _ = write!(
                    body,
                    "<div class=\"event\"><div class=\"role\">USER — {timestamp}</div><div class=\"event-body\">{}</div></div>",
                    html_escape(text)
                );
            }
            TranscriptEvent::AssistantMessage { timestamp, text, .. } => {
                let _ = write!(
                    body,
                    "<div class=\"event\"><div class=\"role\">ASSISTANT — {timestamp}</div><div class=\"event-body\">{}</div></div>",
                    html_escape(text)
                );
            }
            TranscriptEvent::ToolUse { timestamp, name, summary, is_error } => {
                let (mark, dot) = if *is_error {
                    ("✗", "<span class=\"error-dot\"></span>")
                } else {
                    ("▶", "")
                };
                let _ = write!(
                    body,
                    "<details class=\"tool-call\"><summary>{mark} {} — {} ({timestamp}){dot}</summary></details>",
                    html_escape(name),
                    html_escape(summary)
                );
            }
            TranscriptEvent::Marker { timestamp, label } => {
                let _ = write!(body, "<div class=\"marker-row\">— {} ({timestamp}) —</div>", html_escape(label));
            }
        }
    }
    body.push_str("</div>");

    page(&format!("セッション詳細: {}", s.id), &body, false)
}
