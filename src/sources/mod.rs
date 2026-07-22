pub mod claude_code;
pub mod codex;
pub mod cursor;

use crate::model::{Session, Transcript};
use anyhow::Result;
use std::path::{Path, PathBuf};

pub trait Source {
    fn tool(&self) -> crate::model::Tool;

    /// スキャン対象ファイル一覧を返す(実際のパースはしない、キャッシュ判定用)。
    fn discover(&self) -> Result<Vec<PathBuf>>;

    /// 1ファイルをパースしセッション群を返す(キャッシュミス時のみ呼ばれる)。
    fn parse_file(&self, path: &Path) -> Result<Vec<Session>>;

    /// session詳細表示用に生ログを再パースしトランスクリプトを構築する。
    fn parse_transcript(&self, path: &Path, session_id: &str) -> Result<Transcript>;
}
