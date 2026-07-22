use crate::model::{ModelUsage, Session, ToolCallStat, Tool, Transcript, TranscriptEvent, Usage};
use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct CursorSource {
    pub root: PathBuf,
}

impl Default for CursorSource {
    fn default() -> Self {
        let root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cursor")
            .join("chats");
        CursorSource { root }
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

    /// store.db を持つセッションディレクトリ配下の store.db パスを返す。
    fn discover(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        let Ok(workspaces) = std::fs::read_dir(&self.root) else {
            return Ok(out);
        };
        for ws in workspaces.flatten() {
            let ws_path = ws.path();
            if !ws_path.is_dir() {
                continue;
            }
            let Ok(sessions) = std::fs::read_dir(&ws_path) else { continue };
            for sess in sessions.flatten() {
                let sess_path = sess.path();
                let db_path = sess_path.join("store.db");
                if db_path.is_file() {
                    out.push(db_path);
                }
            }
        }
        Ok(out)
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

fn ms_to_datetime(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_else(Utc::now)
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

/// blobs テーブルから role/content を持つ JSON エントリのみ抽出する。
/// (メッセージ間の内部構造を表す protobuf 風バイナリ blob は JSON パースに失敗するため自然にスキップされる)
fn load_role_messages(store_db_path: &Path) -> Vec<RoleMessage> {
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

fn estimate_tokens(chars: usize) -> u64 {
    (chars as u64) / 4
}

fn build_session(path: &Path) -> Result<Option<Session>> {
    if !path.is_file() {
        return Ok(None);
    }
    let meta = load_meta(path);
    let messages = load_role_messages(path);
    if messages.is_empty() && meta.created_at_ms.is_none() {
        return Ok(None);
    }

    let start = meta.created_at_ms.map(ms_to_datetime).unwrap_or_else(Utc::now);
    let end = meta.updated_at_ms.map(ms_to_datetime).unwrap_or(start);

    let turns = messages.iter().filter(|m| m.role == "user").count() as u32;
    let first_prompt = load_first_prompt(path)
        .or_else(|| messages.iter().find(|m| m.role == "user").map(|m| m.text.clone()));

    let total_chars: usize = messages.iter().map(|m| m.text.chars().count()).sum();
    let usage = Usage {
        input_tokens: 0,
        output_tokens: estimate_tokens(total_chars),
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        estimated: true,
    };

    let tool_count = messages.iter().filter(|m| m.role == "tool").count() as u64;
    let tool_calls = if tool_count > 0 {
        vec![ToolCallStat { name: "tool".into(), count: tool_count, error_count: 0 }]
    } else {
        vec![]
    };

    // モデル名は assistant blob の providerOptions.cursor.modelName から取得できるが、
    // トークン usage が取れないためいずれにせよコストは未知単価扱いになる。
    let model_name = messages
        .iter()
        .find_map(|m| m.model.clone())
        .unwrap_or_else(|| "cursor-unknown".into());
    let models = vec![ModelUsage { model: model_name, usage }];

    Ok(Some(Session {
        tool: Tool::Cursor,
        id: session_id_from_path(path),
        source_path: path.to_string_lossy().to_string(),
        repo: meta.cwd.as_deref().map(repo_from_cwd),
        cwd: meta.cwd,
        start,
        end,
        turns,
        first_prompt,
        models,
        usage,
        tool_calls,
        cost: crate::model::Cost::default(),
    }))
}

fn build_events(path: &Path) -> Result<Vec<TranscriptEvent>> {
    let meta = load_meta(path);
    let ts = meta.created_at_ms.map(ms_to_datetime).unwrap_or_else(Utc::now);
    let messages = load_role_messages(path);

    let mut events = Vec::new();
    events.push(TranscriptEvent::Marker { timestamp: ts, label: "セッション開始".into() });
    for m in messages {
        match m.role.as_str() {
            "user" => events.push(TranscriptEvent::UserMessage { timestamp: ts, text: m.text }),
            "assistant" => events.push(TranscriptEvent::AssistantMessage {
                timestamp: ts,
                text: m.text,
                model: Some(m.model.unwrap_or_else(|| "cursor-unknown".into())),
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

    fn write_fixture(dir: &Path) -> PathBuf {
        let sess_dir = dir.join("sess-1");
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

    #[test]
    fn parses_session_summary_with_estimated_usage() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fixture(tmp.path());
        let source = CursorSource::default();
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
    fn builds_transcript_events() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fixture(tmp.path());
        let source = CursorSource::default();
        let id = session_id_from_path(&path);
        let transcript = source.parse_transcript(&path, &id).unwrap();
        assert!(transcript.events.iter().any(|e| matches!(e, TranscriptEvent::UserMessage { .. })));
        assert!(transcript.events.iter().any(|e| matches!(e, TranscriptEvent::AssistantMessage { .. })));
    }
}
