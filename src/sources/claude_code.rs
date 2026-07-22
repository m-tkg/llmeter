use crate::model::{
    ModelUsage, Session, ToolCallStat, Tool, Transcript, TranscriptEvent, Usage,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct ClaudeCodeSource {
    pub root: PathBuf,
}

impl Default for ClaudeCodeSource {
    fn default() -> Self {
        let root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude")
            .join("projects");
        ClaudeCodeSource { root }
    }
}

#[derive(Debug, Deserialize)]
struct RawLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: bool,
    #[serde(rename = "isMeta", default)]
    is_meta: bool,
    timestamp: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    message: Option<serde_json::Value>,
}

impl crate::sources::Source for ClaudeCodeSource {
    fn tool(&self) -> Tool {
        Tool::ClaudeCode
    }

    fn discover(&self) -> Result<Vec<PathBuf>> {
        Ok(walk_jsonl(&self.root))
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<Session>> {
        let Some(builder) = build_session(path)? else {
            return Ok(vec![]);
        };
        Ok(vec![builder])
    }

    fn parse_transcript(&self, path: &Path, session_id: &str) -> Result<Transcript> {
        let session = build_session(path)?
            .filter(|s| s.id == session_id)
            .ok_or_else(|| anyhow::anyhow!("セッションが見つからない: {session_id}"))?;
        let events = build_events(path)?;
        Ok(Transcript { session, events })
    }
}

fn walk_jsonl(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_jsonl(&path));
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            out.push(path);
        }
    }
    out
}

fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

/// meta/コマンド由来のテキストは first_prompt 候補から除外する。
fn is_real_user_text(text: &str) -> bool {
    !text.trim_start().starts_with('<')
}

fn repo_from_cwd(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string())
}

fn build_session(path: &Path) -> Result<Option<Session>> {
    let content = std::fs::read_to_string(path)?;

    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut start: Option<DateTime<Utc>> = None;
    let mut end: Option<DateTime<Utc>> = None;
    let mut turns: u32 = 0;
    let mut first_prompt: Option<String> = None;
    let mut model_usage: HashMap<String, Usage> = HashMap::new();
    let mut tool_calls: HashMap<String, ToolCallStat> = HashMap::new();
    // tool_use_id -> tool name。対応する tool_result の is_error をエラーカウントに反映する。
    let mut pending_tool_uses: HashMap<String, String> = HashMap::new();
    // 同一 API レスポンス(message.id)が thinking/text/tool_use ごとに複数行へ分割記録されるため、
    // usage・turns を二重計上しないよう message.id 単位で1回だけ加算する。
    let mut seen_message_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(raw) = serde_json::from_str::<RawLine>(line) else {
            continue;
        };
        if raw.is_sidechain {
            continue;
        }

        if session_id.is_none() {
            session_id = raw.session_id.clone();
        }
        if cwd.is_none() {
            cwd = raw.cwd.clone();
        }

        let ts = raw.timestamp.as_deref().and_then(parse_timestamp);
        if let Some(ts) = ts {
            start = Some(start.map_or(ts, |s: DateTime<Utc>| s.min(ts)));
            end = Some(end.map_or(ts, |e: DateTime<Utc>| e.max(ts)));
        }

        match raw.kind.as_deref() {
            Some("user") => {
                if let Some(msg) = &raw.message {
                    let content_val = msg.get("content");
                    match content_val {
                        Some(serde_json::Value::String(text)) => {
                            if !raw.is_meta && first_prompt.is_none() && is_real_user_text(text) {
                                first_prompt = Some(text.clone());
                            }
                            if !raw.is_meta {
                                turns += 1;
                            }
                        }
                        Some(serde_json::Value::Array(items)) => {
                            for item in items {
                                if item.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                                    let tool_use_id = item.get("tool_use_id").and_then(|v| v.as_str());
                                    let is_error = item
                                        .get("is_error")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false);
                                    if is_error
                                        && let Some(id) = tool_use_id
                                            && let Some(name) = pending_tool_uses.get(id)
                                                && let Some(stat) = tool_calls.get_mut(name) {
                                                    stat.error_count += 1;
                                                }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("assistant") => {
                if let Some(msg) = &raw.message {
                    let message_id = msg.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let is_new_message = match &message_id {
                        Some(id) => seen_message_ids.insert(id.clone()),
                        None => true,
                    };

                    if is_new_message {
                        turns += 1;
                        let model = msg.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());
                        let usage = msg.get("usage");
                        if let (Some(model), Some(usage)) = (&model, usage) {
                            let u = Usage {
                                input_tokens: usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                output_tokens: usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                cache_creation_tokens: usage
                                    .get("cache_creation_input_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0),
                                cache_read_tokens: usage
                                    .get("cache_read_input_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0),
                                estimated: false,
                            };
                            model_usage.entry(model.clone()).or_default().add(&u);
                        }
                    }

                    if let Some(serde_json::Value::Array(items)) = msg.get("content") {
                        for item in items {
                            if item.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                                let name = item
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let id = item.get("id").and_then(|v| v.as_str());
                                let stat = tool_calls.entry(name.clone()).or_insert_with(|| ToolCallStat {
                                    name: name.clone(),
                                    count: 0,
                                    error_count: 0,
                                });
                                stat.count += 1;
                                if let Some(id) = id {
                                    pending_tool_uses.insert(id.to_string(), name);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let Some(session_id) = session_id else { return Ok(None) };
    let Some(start) = start else { return Ok(None) };
    let end = end.unwrap_or(start);

    let mut usage = Usage::default();
    let models: Vec<ModelUsage> = model_usage
        .into_iter()
        .map(|(model, u)| {
            usage.add(&u);
            ModelUsage { model, usage: u }
        })
        .collect();

    Ok(Some(Session {
        tool: Tool::ClaudeCode,
        id: session_id,
        source_path: path.to_string_lossy().to_string(),
        repo: cwd.as_deref().map(repo_from_cwd),
        cwd,
        start,
        end,
        turns,
        first_prompt,
        models,
        usage,
        tool_calls: tool_calls.into_values().collect(),
        cost: crate::model::Cost::default(),
    }))
}

fn build_events(path: &Path) -> Result<Vec<TranscriptEvent>> {
    let content = std::fs::read_to_string(path)?;
    let mut events = Vec::new();
    let mut marked_start = false;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(raw) = serde_json::from_str::<RawLine>(line) else {
            continue;
        };
        if raw.is_sidechain || raw.is_meta {
            continue;
        }
        let Some(ts) = raw.timestamp.as_deref().and_then(parse_timestamp) else {
            continue;
        };
        if !marked_start {
            events.push(TranscriptEvent::Marker { timestamp: ts, label: "セッション開始".into() });
            marked_start = true;
        }

        match raw.kind.as_deref() {
            Some("user") => {
                if let Some(msg) = &raw.message {
                    match msg.get("content") {
                        Some(serde_json::Value::String(text)) => {
                            if is_real_user_text(text) {
                                events.push(TranscriptEvent::UserMessage { timestamp: ts, text: text.clone() });
                            }
                        }
                        Some(serde_json::Value::Array(items)) => {
                            for item in items {
                                if item.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                                    let is_error = item.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                                    let summary = summarize_tool_result(item);
                                    events.push(TranscriptEvent::ToolUse {
                                        timestamp: ts,
                                        name: "tool_result".into(),
                                        summary,
                                        is_error,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("assistant") => {
                if let Some(msg) = &raw.message {
                    let model = msg.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());
                    if let Some(serde_json::Value::Array(items)) = msg.get("content") {
                        for item in items {
                            match item.get("type").and_then(|v| v.as_str()) {
                                Some("text") => {
                                    if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                                        events.push(TranscriptEvent::AssistantMessage {
                                            timestamp: ts,
                                            text: text.to_string(),
                                            model: model.clone(),
                                        });
                                    }
                                }
                                Some("tool_use") => {
                                    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                                    let input_summary = item
                                        .get("input")
                                        .map(|v| v.to_string())
                                        .unwrap_or_default();
                                    let summary: String = input_summary.chars().take(120).collect();
                                    events.push(TranscriptEvent::ToolUse {
                                        timestamp: ts,
                                        name: name.to_string(),
                                        summary,
                                        is_error: false,
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(events)
}

fn summarize_tool_result(item: &serde_json::Value) -> String {
    match item.get("content") {
        Some(serde_json::Value::String(s)) => s.chars().take(120).collect(),
        Some(other) => other.to_string().chars().take(120).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::Source;

    fn write_fixture(dir: &Path) -> PathBuf {
        let path = dir.join("session.jsonl");
        let lines = [r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>ignore</local-command-caveat>"},"timestamp":"2026-07-01T00:00:00.000Z","cwd":"/Users/masaki/git/github.com/m-tkg/demo","sessionId":"sess-1","uuid":"u0"}"#.to_string(),
            r#"{"type":"user","message":{"role":"user","content":"バグを直して"},"timestamp":"2026-07-01T00:00:01.000Z","cwd":"/Users/masaki/git/github.com/m-tkg/demo","sessionId":"sess-1","uuid":"u1"}"#.to_string(),
            r#"{"type":"assistant","message":{"model":"claude-sonnet-5","content":[{"type":"text","text":"直します"},{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":10,"cache_read_input_tokens":5}},"timestamp":"2026-07-01T00:00:02.000Z","cwd":"/Users/masaki/git/github.com/m-tkg/demo","sessionId":"sess-1","uuid":"a1"}"#.to_string(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":"error: not found","is_error":true}]},"timestamp":"2026-07-01T00:00:03.000Z","cwd":"/Users/masaki/git/github.com/m-tkg/demo","sessionId":"sess-1","uuid":"u2"}"#.to_string(),
            r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-4-5","content":[{"type":"text","text":"サブエージェント出力"}],"usage":{"input_tokens":1,"output_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}},"timestamp":"2026-07-01T00:00:04.000Z","cwd":"/Users/masaki/git/github.com/m-tkg/demo","sessionId":"sess-1","uuid":"a2"}"#.to_string()];
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    #[test]
    fn parses_session_summary_excluding_sidechain() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fixture(tmp.path());
        let source = ClaudeCodeSource::default();
        let sessions = source.parse_file(&path).unwrap();
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.id, "sess-1");
        assert_eq!(s.repo.as_deref(), Some("demo"));
        assert_eq!(s.first_prompt.as_deref(), Some("バグを直して"));
        // isSidechain の assistant 行は turns/usage/models に含まれない
        assert_eq!(s.turns, 2); // 1 user(非meta) + 1 assistant(非sidechain)
        assert_eq!(s.models.len(), 1);
        assert_eq!(s.models[0].model, "claude-sonnet-5");
        assert_eq!(s.usage.input_tokens, 100);
        let bash = s.tool_calls.iter().find(|t| t.name == "Bash").unwrap();
        assert_eq!(bash.count, 1);
        assert_eq!(bash.error_count, 1);
    }

    #[test]
    fn builds_transcript_events() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fixture(tmp.path());
        let source = ClaudeCodeSource::default();
        let transcript = source.parse_transcript(&path, "sess-1").unwrap();
        assert!(transcript.events.iter().any(|e| matches!(e, TranscriptEvent::UserMessage { .. })));
        assert!(transcript.events.iter().any(|e| matches!(e, TranscriptEvent::AssistantMessage { .. })));
    }
}
