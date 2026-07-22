use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const PROMPT: &str = "\
あなたは AI コーディングツールの利用コスト最適化の専門家。stdin から渡される利用レポート（Markdown）を分析し、コスト削減の提案をせよ。

要件:
- 出力は Markdown。見出しは ### 以下のみ使用（# や ## は使わない）
- 「### 分析サマリー」「### コスト削減提案」「### 利用パターンの気づき」の3節構成
- 提案は具体的に。数値（現状コスト・削減見込み）をレポートから引用して根拠を示す
- モデル選択（高価なモデルを使いすぎている作業はないか）、キャッシュ効率、セッションの持ち方、ツールの使い分け、エラー率が高いセッションの傾向、の観点を必ず検討
- 提案は効果が大きい順に最大5件。各提案に「期待削減額/月」の概算を付ける
- 前置き・後書き・謝辞は不要。本文のみ";

/// エージェント名から実行コマンド（プログラム名・引数）を返す。未知エージェントは None。
pub fn agent_command(agent: &str) -> Option<(&'static str, Vec<String>)> {
    match agent {
        "claude" => Some(("claude", vec!["-p".to_string(), PROMPT.to_string()])),
        "codex" => Some(("codex", vec!["exec".to_string(), PROMPT.to_string()])),
        "cursor" => Some(("cursor-agent", vec!["-p".to_string(), PROMPT.to_string()])),
        _ => None,
    }
}

/// エージェント CLI にレポート Markdown を stdin で渡し、分析結果（stdout）を返す。
/// 未インストール・非0終了・タイムアウト・空出力の場合は stderr に警告を出し None を返す
/// （呼び出し側はこれを「分析なし」として扱い、レポート生成自体は継続する）。
pub fn run_agent(agent: &str, input_markdown: &str, timeout_secs: u64) -> Option<String> {
    let Some((program, args)) = agent_command(agent) else {
        eprintln!("警告: 未知のエージェント指定: {agent}");
        return None;
    };

    let mut cmd = Command::new(program);
    cmd.args(&args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("警告: {agent} の起動に失敗した（未インストール？）: {e}");
            return None;
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let input = input_markdown.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
    }

    let mut stdout_pipe = child.stdout.take();
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(s) = stdout_pipe.as_mut() {
            let _ = s.read_to_string(&mut buf);
        }
        buf
    });

    let mut stderr_pipe = child.stderr.take();
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(s) = stderr_pipe.as_mut() {
            let _ = s.read_to_string(&mut buf);
        }
        buf
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if start.elapsed() >= Duration::from_secs(timeout_secs) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => break None,
        }
    };

    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();

    match status {
        None => {
            eprintln!("警告: {agent} の実行がタイムアウトした（{timeout_secs}秒）、分析なしで続行する");
            None
        }
        Some(s) if !s.success() => {
            eprintln!("警告: {agent} が失敗した（{s}）: {}、分析なしで続行する", stderr.trim());
            None
        }
        Some(_) => {
            let trimmed = stdout.trim();
            if trimmed.is_empty() {
                eprintln!("警告: {agent} の出力が空だった、分析なしで続行する");
                None
            } else {
                Some(trimmed.to_string())
            }
        }
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `**text**` / `` `text` `` のような delim で囲まれた区間をタグで置換する。
/// 閉じ delim が見つからない残りはそのまま出力する（壊れた md でも表示が壊れないように）。
fn replace_pairs(s: &str, delim: &str, open_tag: &str, close_tag: &str) -> String {
    let mut result = String::new();
    let mut rest = s;
    while let Some(start) = rest.find(delim) {
        let after_start = &rest[start + delim.len()..];
        if let Some(end) = after_start.find(delim) {
            result.push_str(&rest[..start]);
            result.push_str(open_tag);
            result.push_str(&after_start[..end]);
            result.push_str(close_tag);
            rest = &after_start[end + delim.len()..];
        } else {
            break;
        }
    }
    result.push_str(rest);
    result
}

fn inline_html(s: &str) -> String {
    let escaped = html_escape(s);
    let bolded = replace_pairs(&escaped, "**", "<strong>", "</strong>");
    replace_pairs(&bolded, "`", "<code>", "</code>")
}

/// エージェント出力（Markdown）を最小限の変換で HTML 化する。
/// 対応: `### ` 見出し、`- ` 箇条書き、`**bold**`、`` `code` ``、空行区切り段落。
/// 凝った Markdown パーサには依存しない簡易変換。
pub fn markdown_to_html(md: &str) -> String {
    let mut out = String::new();
    let mut in_list = false;
    let mut para_buf: Vec<String> = Vec::new();

    for raw_line in md.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            flush_paragraph(&mut out, &mut para_buf);
            close_list(&mut out, &mut in_list);
            continue;
        }
        if let Some(rest) = line.strip_prefix("### ") {
            flush_paragraph(&mut out, &mut para_buf);
            close_list(&mut out, &mut in_list);
            out.push_str("<h3>");
            out.push_str(&inline_html(rest));
            out.push_str("</h3>");
            continue;
        }
        if let Some(rest) = line.strip_prefix("- ") {
            flush_paragraph(&mut out, &mut para_buf);
            if !in_list {
                out.push_str("<ul>");
                in_list = true;
            }
            out.push_str("<li>");
            out.push_str(&inline_html(rest));
            out.push_str("</li>");
            continue;
        }
        close_list(&mut out, &mut in_list);
        para_buf.push(inline_html(line));
    }
    flush_paragraph(&mut out, &mut para_buf);
    close_list(&mut out, &mut in_list);

    out
}

fn flush_paragraph(out: &mut String, buf: &mut Vec<String>) {
    if !buf.is_empty() {
        out.push_str("<p>");
        out.push_str(&buf.join("<br>"));
        out.push_str("</p>");
        buf.clear();
    }
}

fn close_list(out: &mut String, in_list: &mut bool) {
    if *in_list {
        out.push_str("</ul>");
        *in_list = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_command_maps_known_agents() {
        let (prog, args) = agent_command("claude").unwrap();
        assert_eq!(prog, "claude");
        assert_eq!(args, vec!["-p".to_string(), PROMPT.to_string()]);

        let (prog, args) = agent_command("codex").unwrap();
        assert_eq!(prog, "codex");
        assert_eq!(args, vec!["exec".to_string(), PROMPT.to_string()]);

        let (prog, args) = agent_command("cursor").unwrap();
        assert_eq!(prog, "cursor-agent");
        assert_eq!(args, vec!["-p".to_string(), PROMPT.to_string()]);
    }

    #[test]
    fn agent_command_unknown_returns_none() {
        assert!(agent_command("chatgpt").is_none());
    }

    #[test]
    fn markdown_to_html_converts_heading() {
        let html = markdown_to_html("### 分析サマリー");
        assert_eq!(html, "<h3>分析サマリー</h3>");
    }

    #[test]
    fn markdown_to_html_converts_list() {
        let html = markdown_to_html("- 提案1\n- 提案2");
        assert_eq!(html, "<ul><li>提案1</li><li>提案2</li></ul>");
    }

    #[test]
    fn markdown_to_html_converts_bold_and_code() {
        let html = markdown_to_html("**重要**な提案。`gpt-5` を使用中。");
        assert_eq!(html, "<p><strong>重要</strong>な提案。<code>gpt-5</code> を使用中。</p>");
    }

    #[test]
    fn markdown_to_html_escapes_html() {
        let html = markdown_to_html("コスト <$100> & 注意");
        assert_eq!(html, "<p>コスト &lt;$100&gt; &amp; 注意</p>");
    }

    #[test]
    fn markdown_to_html_separates_paragraphs_on_blank_line() {
        let html = markdown_to_html("段落1\n\n段落2");
        assert_eq!(html, "<p>段落1</p><p>段落2</p>");
    }

    #[test]
    fn markdown_to_html_mixed_structure() {
        let md = "### 見出し\n\n本文です。\n\n- 項目A\n- 項目B\n\n### 次の見出し";
        let html = markdown_to_html(md);
        assert_eq!(
            html,
            "<h3>見出し</h3><p>本文です。</p><ul><li>項目A</li><li>項目B</li></ul><h3>次の見出し</h3>"
        );
    }
}
