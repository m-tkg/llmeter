//! セッション一覧の「初回プロンプト」表示用テキストの判定・抽出。

const CONTINUATION_PREFIX: &str = "This session is being continued from a previous conversation";

/// 一覧表示に載せるユーザー発話かどうか（厳格: prompt_history 用）。
pub fn is_displayable_user_prompt(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if t.starts_with('<') {
        return false;
    }
    // Cursor / Claude のスラッシュコマンド（/clear, /morning）。/Users/... 等のパスは除外しない。
    if is_slash_command(t) {
        return false;
    }
    if t.starts_with(CONTINUATION_PREFIX) {
        return false;
    }
    true
}

/// `/clear` 等のスラッシュコマンド。ファイルパス `/Users/...` は false。
fn is_slash_command(text: &str) -> bool {
    let line = text.trim().lines().next().unwrap_or("").trim();
    if !line.starts_with('/') || line.starts_with("//") {
        return false;
    }
    let token = line[1..].split_whitespace().next().unwrap_or("");
    !token.is_empty()
        && !token.contains('/')
        && token
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '-' || c == '_')
}

fn extract_tag_inner(text: &str, open: &str, close: &str) -> Option<String> {
    let start = text.find(open)?;
    let after = start + open.len();
    let rel = text[after..].find(close)?;
    let inner = text[after..after + rel].trim();
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_string())
    }
}

/// Cursor store の user blob 内 `Workspace Path:` を取り出す。
pub fn extract_cursor_workspace_path(text: &str) -> Option<String> {
    const KEY: &str = "Workspace Path:";
    if let Some(idx) = text.find(KEY) {
        let rest = text[idx + KEY.len()..].trim_start();
        let path = rest
            .split(|c: char| c == '\n' || c == '<')
            .next()
            .unwrap_or(rest)
            .trim();
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }
    None
}

/// Cursor subagent へ渡す親エージェントの委譲プロンプトっぽい文言。
pub fn is_subagent_delegation_prompt(text: &str) -> bool {
    let t = text.trim();
    t.starts_with("Repository:") && t.contains("Read-only investigation")
}

/// Cursor store の user blob 内 `<user_query>` を取り出す。
pub fn extract_cursor_user_query(text: &str) -> Option<String> {
    extract_tag_inner(text, "<user_query>", "</user_query>")
}

/// Claude Code CLI の `<command-message>...</command-message>` を取り出す。
pub fn extract_claude_command_message(text: &str) -> Option<String> {
    extract_tag_inner(text, "<command-message>", "</command-message>")
}

/// Cursor が user ロールで注入するシステムコンテキスト（人間の入力ではない）。
pub fn is_cursor_injected_context(text: &str) -> bool {
    let t = text.trim();
    t.starts_with("<user_info>")
        || t.contains("<agent_transcripts>")
        || t.contains("<always_applied_workspace_rules")
        || t.contains("<system_reminder>")
        || t.contains("<mcp_instructions>")
}

fn prompt_if_listable(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if t.starts_with(CONTINUATION_PREFIX) {
        return None;
    }
    if is_slash_command(t) {
        return Some(t.lines().next().unwrap_or(t).trim().to_string());
    }
    if t.starts_with('<') {
        return None;
    }
    Some(t.to_string())
}

fn strict_displayable(text: &str) -> Option<String> {
    is_displayable_user_prompt(text).then(|| text.trim().to_string())
}

/// 一覧用: スラッシュコマンドや command-message もラベルとして返す。
pub fn listable_user_prompt(text: &str) -> Option<String> {
    if let Some(q) = extract_cursor_user_query(text) {
        return prompt_if_listable(&q);
    }
    if let Some(msg) = extract_claude_command_message(text) {
        return prompt_if_listable(&msg);
    }
    if is_cursor_injected_context(text) {
        return None;
    }
    prompt_if_listable(text)
}

/// 生テキストを表示用プロンプトに正規化する。
pub fn normalize_user_prompt(text: &str) -> Option<String> {
    listable_user_prompt(text)
}

fn first_displayable_from_text(text: &str) -> Option<String> {
    if let Some(q) = extract_cursor_user_query(text) {
        return strict_displayable(&q);
    }
    if let Some(msg) = extract_claude_command_message(text) {
        return strict_displayable(&msg);
    }
    if is_cursor_injected_context(text) {
        return None;
    }
    strict_displayable(text.trim())
}

/// 候補列から最初の表示可能プロンプトを返す（スラッシュコマンドはスキップ）。
pub fn first_displayable_prompt(candidates: impl IntoIterator<Item = impl AsRef<str>>) -> Option<String> {
    for c in candidates {
        if let Some(p) = first_displayable_from_text(c.as_ref()) {
            return Some(p);
        }
    }
    None
}

/// prompt_history.json 用。Cursor は新しい順に積むため末尾がセッション初回。
pub fn first_chronological_prompt_from_history(candidates: &[String]) -> Option<String> {
    first_displayable_prompt(candidates.iter().rev())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_user_info_and_slash_commands() {
        assert!(!is_displayable_user_prompt("<user_info>OS</user_info>"));
        assert!(!is_displayable_user_prompt("/clear"));
        assert!(!is_displayable_user_prompt("/morning "));
    }

    #[test]
    fn rejects_continuation_summary() {
        assert!(!is_displayable_user_prompt(
            "This session is being continued from a previous conversation that ran out of context."
        ));
    }

    #[test]
    fn rejects_user_info_without_user_query() {
        let blob = "<user_info>\nOS Version: darwin\n</user_info>\n<rules>...</rules>";
        assert!(normalize_user_prompt(blob).is_none());
    }

    #[test]
    fn extracts_user_query_from_cursor_blob() {
        let blob = "<timestamp>...</timestamp>\n<user_query>コンソールからは開けそうだが</user_query>";
        assert_eq!(
            normalize_user_prompt(blob).as_deref(),
            Some("コンソールからは開けそうだが")
        );
    }

    #[test]
    fn extracts_claude_command_message() {
        let xml = "<command-name>/clear</command-name>\n<command-message>migrate-vertex</command-message>";
        assert_eq!(
            normalize_user_prompt(xml).as_deref(),
            Some("migrate-vertex")
        );
    }

    #[test]
    fn listable_includes_slash_commands() {
        assert_eq!(listable_user_prompt("/morning").as_deref(), Some("/morning"));
    }

    #[test]
    fn allows_absolute_paths_as_prompt() {
        assert!(is_displayable_user_prompt(
            "/Users/masaki/work/ai/report.html\nこれとか、"
        ));
    }

    #[test]
    fn history_newest_first_returns_chronological_first() {
        let prompts = vec![
            "直して".to_string(),
            "一旦コミットして".to_string(),
            "グラフについて".to_string(),
        ];
        assert_eq!(
            first_chronological_prompt_from_history(&prompts).as_deref(),
            Some("グラフについて")
        );
    }

    #[test]
    fn extracts_workspace_path_inline_in_user_info() {
        let blob = "<user_info>Workspace Path: /Users/demo/karte-io-systems-ops</user_info>";
        assert_eq!(
            extract_cursor_workspace_path(blob).as_deref(),
            Some("/Users/demo/karte-io-systems-ops")
        );
    }

    #[test]
    fn detects_subagent_delegation_prompt() {
        assert!(is_subagent_delegation_prompt(
            "Repository: /Users/demo/repo. Read-only investigation. Determine exact manifests"
        ));
    }
}
