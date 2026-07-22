use crate::model::{ModelUsage, Session, ToolCallStat, Tool, Transcript, TranscriptEvent, Usage};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct CodexSource {
    pub root: PathBuf,
}

impl Default for CodexSource {
    fn default() -> Self {
        let root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".codex")
            .join("sessions");
        CodexSource { root }
    }
}

impl crate::sources::Source for CodexSource {
    fn tool(&self) -> Tool {
        Tool::Codex
    }

    fn discover(&self) -> Result<Vec<PathBuf>> {
        Ok(walk_jsonl(&self.root))
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<Session>> {
        match build_session(path)? {
            Some(s) => Ok(vec![s]),
            None => Ok(vec![]),
        }
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

fn repo_from_cwd(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string())
}

/// function_call_output の "Process exited with code N" を見て N!=0 ならエラー扱いにする。
fn output_is_error(output: &str) -> bool {
    if let Some(idx) = output.find("Process exited with code ") {
        let rest = &output[idx + "Process exited with code ".len()..];
        let code_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = code_str.parse::<i64>() {
            return code != 0;
        }
    }
    false
}

struct ParsedLine {
    ts: Option<DateTime<Utc>>,
    kind: String,
    payload: Value,
}

fn read_lines(path: &Path) -> Result<Vec<ParsedLine>> {
    let content = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let ts = v.get("timestamp").and_then(|t| t.as_str()).and_then(parse_timestamp);
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
        let payload = v.get("payload").cloned().unwrap_or(Value::Null);
        out.push(ParsedLine { ts, kind, payload });
    }
    Ok(out)
}

fn build_session(path: &Path) -> Result<Option<Session>> {
    let lines = read_lines(path)?;

    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;
    let mut start: Option<DateTime<Utc>> = None;
    let mut end: Option<DateTime<Utc>> = None;
    let mut turns: u32 = 0;
    let mut first_prompt: Option<String> = None;
    let mut last_total_usage: Option<Usage> = None;

    let mut call_id_to_name: HashMap<String, String> = HashMap::new();
    let mut tool_calls: HashMap<String, ToolCallStat> = HashMap::new();
    // call_id -> is_error (function_call_output を先に集める2パス目のために使う)
    let mut call_id_error: HashMap<String, bool> = HashMap::new();

    for line in &lines {
        if let Some(ts) = line.ts {
            start = Some(start.map_or(ts, |s: DateTime<Utc>| s.min(ts)));
            end = Some(end.map_or(ts, |e: DateTime<Utc>| e.max(ts)));
        }

        match line.kind.as_str() {
            "session_meta" => {
                session_id = line.payload.get("id").and_then(|v| v.as_str()).map(String::from);
                cwd = line.payload.get("cwd").and_then(|v| v.as_str()).map(String::from);
            }
            "turn_context" => {
                if let Some(m) = line.payload.get("model").and_then(|v| v.as_str()) {
                    model = Some(m.to_string());
                }
            }
            "event_msg" => {
                let etype = line.payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match etype {
                    "user_message" => {
                        turns += 1;
                        if first_prompt.is_none()
                            && let Some(text) = line.payload.get("message").and_then(|v| v.as_str()) {
                                first_prompt = Some(text.to_string());
                            }
                    }
                    "token_count" => {
                        if let Some(info) = line.payload.get("info")
                            && let Some(total) = info.get("total_token_usage") {
                                // input_tokens は cached_input_tokens を含む値なので、
                                // cache_read 分と二重計上しないよう差し引く。
                                let input_total = total.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                                let cached = total.get("cached_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                                let u = Usage {
                                    input_tokens: input_total.saturating_sub(cached),
                                    output_tokens: total.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                    cache_creation_tokens: 0,
                                    cache_read_tokens: cached,
                                    estimated: false,
                                };
                                last_total_usage = Some(u);
                            }
                    }
                    _ => {}
                }
            }
            "response_item" => {
                let rtype = line.payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match rtype {
                    "function_call" => {
                        let name = line.payload.get("name").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                        let call_id = line.payload.get("call_id").and_then(|v| v.as_str()).map(String::from);
                        let stat = tool_calls.entry(name.clone()).or_insert_with(|| ToolCallStat {
                            name: name.clone(),
                            count: 0,
                            error_count: 0,
                        });
                        stat.count += 1;
                        if let Some(id) = call_id {
                            call_id_to_name.insert(id, name);
                        }
                    }
                    "function_call_output" => {
                        let call_id = line.payload.get("call_id").and_then(|v| v.as_str());
                        let output = line.payload.get("output").and_then(|v| v.as_str()).unwrap_or("");
                        let is_error = output_is_error(output);
                        if let Some(id) = call_id {
                            call_id_error.insert(id.to_string(), is_error);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    for (call_id, is_error) in &call_id_error {
        if *is_error
            && let Some(name) = call_id_to_name.get(call_id)
                && let Some(stat) = tool_calls.get_mut(name) {
                    stat.error_count += 1;
                }
    }

    let Some(session_id) = session_id else { return Ok(None) };
    let Some(start) = start else { return Ok(None) };
    let end = end.unwrap_or(start);

    let usage = last_total_usage.unwrap_or_default();
    let models = model
        .map(|m| vec![ModelUsage { model: m, usage }])
        .unwrap_or_default();

    Ok(Some(Session {
        tool: Tool::Codex,
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
    let lines = read_lines(path)?;

    // 2パス目用: call_id -> is_error
    let mut call_id_error: HashMap<String, bool> = HashMap::new();
    for line in &lines {
        if line.kind == "response_item" && line.payload.get("type").and_then(|v| v.as_str()) == Some("function_call_output")
            && let Some(id) = line.payload.get("call_id").and_then(|v| v.as_str()) {
                let output = line.payload.get("output").and_then(|v| v.as_str()).unwrap_or("");
                call_id_error.insert(id.to_string(), output_is_error(output));
            }
    }

    let mut current_model: Option<String> = None;
    let mut events = Vec::new();
    let mut marked_start = false;

    for line in &lines {
        let Some(ts) = line.ts else { continue };
        if !marked_start {
            events.push(TranscriptEvent::Marker { timestamp: ts, label: "セッション開始".into() });
            marked_start = true;
        }
        match line.kind.as_str() {
            "turn_context" => {
                if let Some(m) = line.payload.get("model").and_then(|v| v.as_str()) {
                    current_model = Some(m.to_string());
                }
            }
            "event_msg" => {
                if line.payload.get("type").and_then(|v| v.as_str()) == Some("user_message")
                    && let Some(text) = line.payload.get("message").and_then(|v| v.as_str()) {
                        events.push(TranscriptEvent::UserMessage { timestamp: ts, text: text.to_string() });
                    }
            }
            "response_item" => {
                let rtype = line.payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match rtype {
                    "message" => {
                        if line.payload.get("role").and_then(|v| v.as_str()) == Some("assistant")
                            && let Some(Value::Array(items)) = line.payload.get("content") {
                                for item in items {
                                    if item.get("type").and_then(|v| v.as_str()) == Some("output_text")
                                        && let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                                            events.push(TranscriptEvent::AssistantMessage {
                                                timestamp: ts,
                                                text: text.to_string(),
                                                model: current_model.clone(),
                                            });
                                        }
                                }
                            }
                    }
                    "function_call" => {
                        let name = line.payload.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                        let args = line.payload.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                        let call_id = line.payload.get("call_id").and_then(|v| v.as_str());
                        let is_error = call_id.and_then(|id| call_id_error.get(id)).copied().unwrap_or(false);
                        let summary: String = args.chars().take(120).collect();
                        events.push(TranscriptEvent::ToolUse {
                            timestamp: ts,
                            name: name.to_string(),
                            summary,
                            is_error,
                        });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::Source;

    fn write_fixture(dir: &Path) -> PathBuf {
        let path = dir.join("rollout-test.jsonl");
        let lines = [r#"{"timestamp":"2026-06-12T06:43:24.819Z","type":"session_meta","payload":{"id":"codex-sess-1","cwd":"/Users/masaki/git/github.com/m-tkg/demo","model_provider":"openai"}}"#.to_string(),
            r#"{"timestamp":"2026-06-12T06:43:24.820Z","type":"turn_context","payload":{"turn_id":"t1","model":"gpt-5.5"}}"#.to_string(),
            r#"{"timestamp":"2026-06-12T06:43:24.821Z","type":"event_msg","payload":{"type":"user_message","message":"直して"}}"#.to_string(),
            r#"{"timestamp":"2026-06-12T06:43:25.000Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"ls\"}","call_id":"call1"}}"#.to_string(),
            r#"{"timestamp":"2026-06-12T06:43:25.100Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call1","output":"Process exited with code 1\nerror"}}"#.to_string(),
            r#"{"timestamp":"2026-06-12T06:43:26.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":10,"output_tokens":20}}}}"#.to_string(),
            r#"{"timestamp":"2026-06-12T06:43:27.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"直しました"}]}}"#.to_string()];
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    #[test]
    fn parses_session_summary() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fixture(tmp.path());
        let source = CodexSource::default();
        let sessions = source.parse_file(&path).unwrap();
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.id, "codex-sess-1");
        assert_eq!(s.repo.as_deref(), Some("demo"));
        assert_eq!(s.first_prompt.as_deref(), Some("直して"));
        assert_eq!(s.turns, 1);
        assert_eq!(s.models.len(), 1);
        assert_eq!(s.models[0].model, "gpt-5.5");
        assert_eq!(s.usage.input_tokens, 90);
        assert_eq!(s.usage.cache_read_tokens, 10);
        let call = s.tool_calls.iter().find(|t| t.name == "exec_command").unwrap();
        assert_eq!(call.count, 1);
        assert_eq!(call.error_count, 1);
    }

    #[test]
    fn builds_transcript_events() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fixture(tmp.path());
        let source = CodexSource::default();
        let transcript = source.parse_transcript(&path, "codex-sess-1").unwrap();
        assert!(transcript.events.iter().any(|e| matches!(e, TranscriptEvent::UserMessage { .. })));
        assert!(transcript.events.iter().any(|e| matches!(e, TranscriptEvent::AssistantMessage { .. })));
        let tool_ev = transcript.events.iter().find(|e| matches!(e, TranscriptEvent::ToolUse { .. })).unwrap();
        if let TranscriptEvent::ToolUse { is_error, .. } = tool_ev {
            assert!(*is_error);
        }
    }

    #[test]
    fn output_is_error_detects_nonzero_exit() {
        assert!(output_is_error("Process exited with code 1\nfoo"));
        assert!(!output_is_error("Process exited with code 0\nok"));
        assert!(!output_is_error("no exit code info"));
    }
}
