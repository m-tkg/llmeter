use crate::daily::split_usage_by_duration;
use crate::model::{ModelUsage, Session, ToolCallStat, Tool, Transcript, TranscriptEvent, Usage};
use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct CursorSource {
    pub chats_root: PathBuf,
    pub projects_root: PathBuf,
}

impl Default for CursorSource {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        CursorSource {
            chats_root: home.join(".cursor").join("chats"),
            projects_root: home.join(".cursor").join("projects"),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct Meta {
    #[serde(rename = "createdAtMs")]
    created_at_ms: Option<i64>,
    #[serde(rename = "updatedAtMs")]
    updated_at_ms: Option<i64>,
    cwd: Option<String>,
}

impl crate::sources::Source for CursorSource {
    fn tool(&self) -> Tool {
        Tool::Cursor
    }

    /// store.db と、chats に無い agent-transcripts/*.jsonl を返す。
    fn discover(&self) -> Result<Vec<PathBuf>> {
        let mut out = discover_chat_store_dbs(&self.chats_root);
        let skip_ids = chat_store_session_ids(&self.chats_root);
        out.extend(discover_agent_transcripts(&self.projects_root, &skip_ids));
        Ok(out)
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<Session>> {
        let session = if is_agent_transcript(path) {
            build_session_from_transcript(path)?
        } else {
            build_session_from_store(path)?
        };
        match session {
            Some(s) => Ok(vec![s]),
            None => Ok(vec![]),
        }
    }

    fn parse_transcript(&self, path: &Path, session_id: &str) -> Result<Transcript> {
        let session = if is_agent_transcript(path) {
            build_session_from_transcript(path)?
        } else {
            build_session_from_store(path)?
        }
        .filter(|s| s.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("セッションが見つからない: {session_id}"))?;
        let events = if is_agent_transcript(path) {
            build_transcript_events_from_transcript(path)?
        } else {
            build_events_from_store(path)?
        };
        Ok(Transcript { session, events })
    }
}

fn is_agent_transcript(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "jsonl")
        && path
            .parent()
            .is_some_and(|p| p.ends_with("agent-transcripts") || p.parent().is_some_and(|gp| gp.ends_with("agent-transcripts")))
}

fn discover_chat_store_dbs(chats_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(workspaces) = std::fs::read_dir(chats_root) else {
        return out;
    };
    for ws in workspaces.flatten() {
        let ws_path = ws.path();
        if !ws_path.is_dir() {
            continue;
        }
        let Ok(sessions) = std::fs::read_dir(&ws_path) else { continue };
        for sess in sessions.flatten() {
            let db_path = sess.path().join("store.db");
            if db_path.is_file() {
                out.push(db_path);
            }
        }
    }
    out
}

fn chat_store_session_ids(chats_root: &Path) -> HashSet<String> {
    let mut ids = HashSet::new();
    let Ok(workspaces) = std::fs::read_dir(chats_root) else {
        return ids;
    };
    for ws in workspaces.flatten() {
        let ws_path = ws.path();
        if !ws_path.is_dir() {
            continue;
        }
        let Ok(sessions) = std::fs::read_dir(&ws_path) else { continue };
        for sess in sessions.flatten() {
            let sess_path = sess.path();
            if sess_path.join("store.db").is_file() {
                if let Some(id) = sess_path.file_name().and_then(|n| n.to_str()) {
                    ids.insert(id.to_string());
                }
            }
        }
    }
    ids
}

fn discover_agent_transcripts(projects_root: &Path, skip_ids: &HashSet<String>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    collect_agent_transcripts(projects_root, skip_ids, &mut seen, &mut out);
    out
}

fn collect_agent_transcripts(
    dir: &Path,
    skip_ids: &HashSet<String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_agent_transcripts(&path, skip_ids, seen, out);
            continue;
        }
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        if !path.to_string_lossy().contains("/agent-transcripts/") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if skip_ids.contains(id) || !seen.insert(id.to_string()) {
            continue;
        }
        out.push(path);
    }
}

fn ms_to_datetime(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_else(Utc::now)
}

fn system_time_to_datetime(t: SystemTime) -> DateTime<Utc> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| Utc.timestamp_opt(d.as_secs() as i64, d.subsec_nanos()).single())
        .ok()
        .flatten()
        .unwrap_or_else(Utc::now)
}

fn file_time_range(path: &Path) -> (DateTime<Utc>, DateTime<Utc>) {
    let Ok(meta) = std::fs::metadata(path) else {
        let now = Utc::now();
        return (now, now);
    };
    let modified = meta
        .modified()
        .map(system_time_to_datetime)
        .unwrap_or_else(|_| Utc::now());
    let created = meta.created().map(system_time_to_datetime).unwrap_or(modified);
    (created, modified)
}

fn repo_from_cwd(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string())
}

fn session_dir(store_db_path: &Path) -> &Path {
    store_db_path.parent().unwrap_or(store_db_path)
}

fn session_id_from_path(store_db_path: &Path) -> String {
    session_dir(store_db_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn load_meta(store_db_path: &Path) -> Meta {
    let path = session_dir(store_db_path).join("meta.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn load_first_prompt(store_db_path: &Path) -> Option<String> {
    let path = session_dir(store_db_path).join("prompt_history.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let arr: Vec<String> = serde_json::from_str(&raw).ok()?;
    arr.into_iter().next()
}

struct RoleMessage {
    role: String,
    text: String,
    model: Option<String>,
}

/// assistant blob の content[].providerOptions.cursor.modelName から実モデル名を取り出す。
fn extract_model_name(content: Option<&Value>) -> Option<String> {
    let items = content?.as_array()?;
    for item in items {
        if let Some(name) = item
            .get("providerOptions")
            .and_then(|p| p.get("cursor"))
            .and_then(|c| c.get("modelName"))
            .and_then(|m| m.as_str())
        {
            return Some(name.to_string());
        }
    }
    None
}

fn extract_content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn count_tool_uses(content: Option<&Value>) -> u64 {
    let Some(Value::Array(items)) = content else {
        return 0;
    };
    items
        .iter()
        .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
        .count() as u64
}

fn estimate_tokens(chars: usize) -> u64 {
    (chars as u64) / 4
}

fn role_from_value(v: &Value) -> Option<&str> {
    if let Some(role) = v.get("role").and_then(|r| r.as_str()) {
        return Some(role);
    }
    v.get("message")
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
}

fn content_from_value(v: &Value) -> Option<&Value> {
    if v.get("content").is_some() {
        return v.get("content");
    }
    v.get("message").and_then(|m| m.get("content"))
}

/// blobs テーブルから role/content を持つ JSON エントリのみ抽出する。
fn load_role_messages_from_store(store_db_path: &Path) -> Vec<RoleMessage> {
    let mut out = Vec::new();
    let Ok(conn) = rusqlite::Connection::open(store_db_path) else {
        return out;
    };
    let Ok(mut stmt) = conn.prepare("SELECT data FROM blobs") else {
        return out;
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0)) else {
        return out;
    };

    for row in rows.flatten() {
        let Ok(text) = String::from_utf8(row) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let Some(role) = v.get("role").and_then(|r| r.as_str()) else { continue };
        if role != "user" && role != "assistant" && role != "tool" {
            continue;
        }
        let content_text = extract_content_text(v.get("content"));
        let model = if role == "assistant" { extract_model_name(v.get("content")) } else { None };
        out.push(RoleMessage { role: role.to_string(), text: content_text, model });
    }
    out
}

fn load_role_messages_from_transcript(path: &Path) -> Vec<RoleMessage> {
    let mut out = Vec::new();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("type").and_then(|t| t.as_str()) == Some("turn_ended") {
            continue;
        }
        let Some(role) = role_from_value(&v) else { continue };
        if role != "user" && role != "assistant" && role != "tool" {
            continue;
        }
        let content = content_from_value(&v);
        let content_text = extract_content_text(content);
        let model = if role == "assistant" { extract_model_name(content) } else { None };
        out.push(RoleMessage { role: role.to_string(), text: content_text, model });
    }
    out
}

fn count_tool_uses_in_transcript(path: &Path) -> u64 {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return 0;
    };
    let mut count = 0u64;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if role_from_value(&v) == Some("assistant") {
            count += count_tool_uses(content_from_value(&v));
        }
    }
    count
}

fn project_dir_from_transcript(path: &Path) -> Option<PathBuf> {
    let mut cur = path.parent()?.to_path_buf();
    loop {
        if cur.file_name().is_some_and(|n| n == "agent-transcripts") {
            return cur.parent().map(|p| p.to_path_buf());
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

fn workspace_path_from_project_dir(project_dir: &Path) -> Option<String> {
    let log_path = project_dir.join("worker.log");
    let raw = std::fs::read_to_string(log_path).ok()?;
    for line in raw.lines() {
        if let Some(rest) = line.split("workspacePath=").nth(1) {
            let path = rest.trim();
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }
    None
}

fn cwd_from_transcript(path: &Path) -> Option<String> {
    project_dir_from_transcript(path).and_then(|dir| workspace_path_from_project_dir(&dir))
}

fn session_from_messages(
    id: String,
    source_path: String,
    cwd: Option<String>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    messages: Vec<RoleMessage>,
    tool_count: u64,
    first_prompt: Option<String>,
) -> Option<Session> {
    if messages.is_empty() {
        return None;
    }

    let turns = messages.iter().filter(|m| m.role == "user").count() as u32;
    let first_prompt = first_prompt.or_else(|| messages.iter().find(|m| m.role == "user").map(|m| m.text.clone()));

    let total_chars: usize = messages.iter().map(|m| m.text.chars().count()).sum();
    let usage = Usage {
        input_tokens: 0,
        output_tokens: estimate_tokens(total_chars),
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        estimated: true,
    };

    let tool_calls = if tool_count > 0 {
        vec![ToolCallStat { name: "tool".into(), count: tool_count, error_count: 0 }]
    } else {
        vec![]
    };

    let model_name = messages
        .iter()
        .find_map(|m| m.model.clone())
        .unwrap_or_else(|| "cursor-unknown".into());
    let models = vec![ModelUsage { model: model_name, usage }];

    let daily_models = if start.date_naive() == end.date_naive() {
        let mut m = BTreeMap::new();
        m.insert(start.date_naive(), models.clone());
        m
    } else {
        split_usage_by_duration(start, end, &models)
    };

    Some(Session {
        tool: Tool::Cursor,
        id,
        source_path,
        repo: cwd.as_deref().map(repo_from_cwd),
        cwd,
        start,
        end,
        turns,
        first_prompt,
        models,
        usage,
        tool_calls,
        cost: crate::model::Cost::default(),
        daily_models,
        daily_cost: BTreeMap::new(),
    })
}

fn build_session_from_store(path: &Path) -> Result<Option<Session>> {
    if !path.is_file() {
        return Ok(None);
    }
    let meta = load_meta(path);
    let messages = load_role_messages_from_store(path);
    if messages.is_empty() && meta.created_at_ms.is_none() {
        return Ok(None);
    }

    let start = meta.created_at_ms.map(ms_to_datetime).unwrap_or_else(Utc::now);
    let end = meta.updated_at_ms.map(ms_to_datetime).unwrap_or(start);
    let first_prompt = load_first_prompt(path);
    let tool_count = messages.iter().filter(|m| m.role == "tool").count() as u64;

    Ok(session_from_messages(
        session_id_from_path(path),
        path.to_string_lossy().to_string(),
        meta.cwd,
        start,
        end,
        messages,
        tool_count,
        first_prompt,
    ))
}

fn build_session_from_transcript(path: &Path) -> Result<Option<Session>> {
    if !path.is_file() {
        return Ok(None);
    }
    let messages = load_role_messages_from_transcript(path);
    if messages.is_empty() {
        return Ok(None);
    }
    let (start, end) = file_time_range(path);
    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let tool_count = count_tool_uses_in_transcript(path);
    Ok(session_from_messages(
        id,
        path.to_string_lossy().to_string(),
        cwd_from_transcript(path),
        start,
        end,
        messages,
        tool_count,
        None,
    ))
}

fn build_events_from_store(path: &Path) -> Result<Vec<TranscriptEvent>> {
    let meta = load_meta(path);
    let ts = meta.created_at_ms.map(ms_to_datetime).unwrap_or_else(Utc::now);
    build_events(&load_role_messages_from_store(path), ts)
}

fn build_transcript_events_from_transcript(path: &Path) -> Result<Vec<TranscriptEvent>> {
    let (start, _) = file_time_range(path);
    build_events(&load_role_messages_from_transcript(path), start)
}

fn build_events(messages: &[RoleMessage], ts: DateTime<Utc>) -> Result<Vec<TranscriptEvent>> {
    let mut events = Vec::new();
    events.push(TranscriptEvent::Marker { timestamp: ts, label: "セッション開始".into() });
    for m in messages {
        match m.role.as_str() {
            "user" => events.push(TranscriptEvent::UserMessage { timestamp: ts, text: m.text.clone() }),
            "assistant" => events.push(TranscriptEvent::AssistantMessage {
                timestamp: ts,
                text: m.text.clone(),
                model: Some(m.model.clone().unwrap_or_else(|| "cursor-unknown".into())),
            }),
            "tool" => events.push(TranscriptEvent::ToolUse {
                timestamp: ts,
                name: "tool".into(),
                summary: m.text.chars().take(120).collect(),
                is_error: false,
            }),
            _ => {}
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::Source;

    fn write_store_fixture(dir: &Path) -> PathBuf {
        let sess_dir = dir.join("chats").join("ws").join("sess-1");
        std::fs::create_dir_all(&sess_dir).unwrap();
        std::fs::write(
            sess_dir.join("meta.json"),
            r#"{"schemaVersion":1,"createdAtMs":1784606311186,"hasConversation":true,"title":"t","updatedAtMs":1784606330932,"cwd":"/Users/masaki/git/github.com/m-tkg/demo"}"#,
        )
        .unwrap();
        std::fs::write(sess_dir.join("prompt_history.json"), r#"["ファイルaを削除して"]"#).unwrap();

        let db_path = sess_dir.join("store.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB)", []).unwrap();
        conn.execute(
            "INSERT INTO blobs (id, data) VALUES (?1, ?2)",
            rusqlite::params!["b1", r#"{"role":"user","content":"ファイルaを削除して"}"#.as_bytes()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO blobs (id, data) VALUES (?1, ?2)",
            rusqlite::params!["b2", r#"{"role":"assistant","content":[{"type":"text","text":"削除しました"}]}"#.as_bytes()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO blobs (id, data) VALUES (?1, ?2)",
            rusqlite::params!["b3", &[0xffu8, 0xfe, 0x00, 0x01][..]],
        )
        .unwrap();
        db_path
    }

    fn write_transcript_fixture(dir: &Path, session_id: &str) -> PathBuf {
        let project = dir.join("projects").join("Users-demo-app");
        let transcript_dir = project.join("agent-transcripts").join(session_id);
        std::fs::create_dir_all(&transcript_dir).unwrap();
        std::fs::write(
            project.join("worker.log"),
            "[info] Getting tree structure for workspacePath=/Users/demo/app\n",
        )
        .unwrap();
        let path = transcript_dir.join(format!("{session_id}.jsonl"));
        std::fs::write(
            &path,
            r#"{"role":"user","message":{"content":[{"type":"text","text":"hello"}]}}
{"role":"assistant","message":{"content":[{"type":"text","text":"hi there"},{"type":"tool_use","name":"Read","input":{"path":"a.txt"}}]}}
{"type":"turn_ended","status":"success"}
"#,
        )
        .unwrap();
        path
    }

    #[test]
    fn parses_session_summary_with_estimated_usage() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_store_fixture(tmp.path());
        let source = CursorSource {
            chats_root: tmp.path().join("chats"),
            projects_root: tmp.path().join("projects"),
        };
        let sessions = source.parse_file(&path).unwrap();
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.repo.as_deref(), Some("demo"));
        assert_eq!(s.first_prompt.as_deref(), Some("ファイルaを削除して"));
        assert_eq!(s.turns, 1);
        assert!(s.usage.estimated);
        assert!(s.usage.output_tokens > 0);
    }

    #[test]
    fn parses_agent_transcript_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_transcript_fixture(tmp.path(), "aaaa-bbbb-cccc");
        let source = CursorSource {
            chats_root: tmp.path().join("chats"),
            projects_root: tmp.path().join("projects"),
        };
        let sessions = source.parse_file(&path).unwrap();
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.id, "aaaa-bbbb-cccc");
        assert_eq!(s.repo.as_deref(), Some("app"));
        assert_eq!(s.cwd.as_deref(), Some("/Users/demo/app"));
        assert_eq!(s.turns, 1);
        assert_eq!(s.tool_calls[0].count, 1);
    }

    #[test]
    fn discover_skips_transcript_when_store_db_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let store_path = write_store_fixture(tmp.path());
        let transcript_path = write_transcript_fixture(tmp.path(), "sess-1");
        let source = CursorSource {
            chats_root: tmp.path().join("chats"),
            projects_root: tmp.path().join("projects"),
        };
        let discovered = source.discover().unwrap();
        assert!(discovered.iter().any(|p| p == &store_path));
        assert!(!discovered.iter().any(|p| p == &transcript_path));
    }

    #[test]
    fn builds_transcript_events() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_store_fixture(tmp.path());
        let source = CursorSource {
            chats_root: tmp.path().join("chats"),
            projects_root: tmp.path().join("projects"),
        };
        let id = session_id_from_path(&path);
        let transcript = source.parse_transcript(&path, &id).unwrap();
        assert!(transcript.events.iter().any(|e| matches!(e, TranscriptEvent::UserMessage { .. })));
        assert!(transcript.events.iter().any(|e| matches!(e, TranscriptEvent::AssistantMessage { .. })));
    }
}
