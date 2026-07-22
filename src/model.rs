use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tool {
    ClaudeCode,
    Codex,
    Cursor,
}

impl Tool {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tool::ClaudeCode => "claude-code",
            Tool::Codex => "codex",
            Tool::Cursor => "cursor",
        }
    }

    pub fn parse(s: &str) -> Option<Tool> {
        match s {
            "claude-code" | "claude" => Some(Tool::ClaudeCode),
            "codex" => Some(Tool::Codex),
            "cursor" => Some(Tool::Cursor),
            _ => None,
        }
    }
}

/// トークン使用量。集計時は加算していく。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    /// 実測でなく文字数等からの概算の場合 true（Cursor 等）
    pub estimated: bool,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_creation_tokens + self.cache_read_tokens
    }

    pub fn add(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.estimated = self.estimated || other.estimated;
    }
}

/// セッション中に使われたモデル別の使用量。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsage {
    pub model: String,
    pub usage: Usage,
}

/// ツール呼び出し1件の集計情報（詳細な引数等は保持しない = キャッシュ軽量化)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallStat {
    pub name: String,
    pub count: u64,
    pub error_count: u64,
}

/// コスト。未知モデルが混ざっている場合 unknown_usage に分離。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Cost {
    pub amount_usd: f64,
    /// true の場合 amount_usd は未知モデル分を含まない不完全な値
    pub has_unknown: bool,
}

/// 正規化されたセッション。cache.rs によりディスクキャッシュされる集計単位。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub tool: Tool,
    pub id: String,
    /// 元ログファイルの絶対パス（session 詳細再パース時に使う）
    pub source_path: String,
    pub cwd: Option<String>,
    pub repo: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub turns: u32,
    pub first_prompt: Option<String>,
    pub models: Vec<ModelUsage>,
    pub usage: Usage,
    pub tool_calls: Vec<ToolCallStat>,
    pub cost: Cost,
}

impl Session {
    pub fn duration_secs(&self) -> i64 {
        (self.end - self.start).num_seconds().max(0)
    }

    pub fn tool_call_total(&self) -> u64 {
        self.tool_calls.iter().map(|t| t.count).sum()
    }

    pub fn tool_error_total(&self) -> u64 {
        self.tool_calls.iter().map(|t| t.error_count).sum()
    }

    pub fn tool_error_rate(&self) -> f64 {
        let total = self.tool_call_total();
        if total == 0 {
            0.0
        } else {
            self.tool_error_total() as f64 / total as f64
        }
    }
}

/// セッション詳細ビュー用のイベント（生ログ再パース時のみ構築、キャッシュしない）
#[derive(Debug, Clone)]
pub enum TranscriptEvent {
    UserMessage { timestamp: DateTime<Utc>, text: String },
    AssistantMessage { timestamp: DateTime<Utc>, text: String, model: Option<String> },
    ToolUse { timestamp: DateTime<Utc>, name: String, summary: String, is_error: bool },
    Marker { timestamp: DateTime<Utc>, label: String },
}

#[derive(Debug, Clone)]
pub struct Transcript {
    pub session: Session,
    pub events: Vec<TranscriptEvent>,
}
