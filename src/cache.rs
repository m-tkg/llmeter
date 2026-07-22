use crate::model::Session;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 1 ソースファイルの集計結果のキャッシュエントリ。
/// 1ファイルから複数セッションが取れる可能性(将来のCursor等)を考慮し Vec で保持する。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    source_path: String,
    mtime_secs: i64,
    size: u64,
    sessions: Vec<Session>,
}

pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn open() -> Result<Self> {
        let dir = default_cache_dir();
        fs::create_dir_all(&dir).with_context(|| format!("キャッシュディレクトリ作成失敗: {dir:?}"))?;
        Ok(Cache { dir })
    }

    #[cfg(test)]
    pub fn with_dir(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(Cache { dir })
    }

    fn key_path(&self, source_path: &Path) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(source_path.to_string_lossy().as_bytes());
        let hash = hasher.finalize();
        self.dir.join(format!("{:x}.json", hash))
    }

    /// キャッシュが有効(mtime/sizeが一致)なら Session 群を返す。
    pub fn get(&self, source_path: &Path) -> Result<Option<Vec<Session>>> {
        let meta = match fs::metadata(source_path) {
            Ok(m) => m,
            Err(_) => return Ok(None),
        };
        let path = self.key_path(source_path);
        let raw = match fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        let entry: CacheEntry = match serde_json::from_str(&raw) {
            Ok(e) => e,
            Err(_) => return Ok(None),
        };

        let mtime_secs = mtime_to_secs(meta.modified()?);
        if entry.mtime_secs == mtime_secs && entry.size == meta.len() {
            Ok(Some(entry.sessions))
        } else {
            Ok(None)
        }
    }

    pub fn put(&self, source_path: &Path, sessions: Vec<Session>) -> Result<()> {
        let meta = fs::metadata(source_path)?;
        let entry = CacheEntry {
            source_path: source_path.to_string_lossy().to_string(),
            mtime_secs: mtime_to_secs(meta.modified()?),
            size: meta.len(),
            sessions,
        };
        let path = self.key_path(source_path);
        let raw = serde_json::to_string(&entry)?;
        fs::write(path, raw)?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        if self.dir.exists() {
            for entry in fs::read_dir(&self.dir)? {
                let entry = entry?;
                if entry.path().extension().is_some_and(|e| e == "json") {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(())
    }

    pub fn status(&self) -> Result<(u64, u64)> {
        let mut count = 0u64;
        let mut total_size = 0u64;
        if self.dir.exists() {
            for entry in fs::read_dir(&self.dir)? {
                let entry = entry?;
                if entry.path().extension().is_some_and(|e| e == "json") {
                    count += 1;
                    total_size += entry.metadata()?.len();
                }
            }
        }
        Ok((count, total_size))
    }
}

fn mtime_to_secs(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("llmeter")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cost, Tool, Usage};
    use chrono::Utc;

    fn sample_session(id: &str) -> Session {
        Session {
            tool: Tool::ClaudeCode,
            id: id.to_string(),
            source_path: "dummy".into(),
            cwd: None,
            repo: None,
            start: Utc::now(),
            end: Utc::now(),
            turns: 1,
            first_prompt: Some("hi".into()),
            models: vec![],
            usage: Usage::default(),
            tool_calls: vec![],
            cost: Cost::default(),
        }
    }

    #[test]
    fn roundtrip_hit_and_invalidate_on_change() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        let cache = Cache::with_dir(cache_dir).unwrap();

        let src = tmp.path().join("source.jsonl");
        fs::write(&src, "hello").unwrap();

        assert!(cache.get(&src).unwrap().is_none());

        cache.put(&src, vec![sample_session("s1")]).unwrap();
        let hit = cache.get(&src).unwrap().unwrap();
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].id, "s1");

        // 内容変更(=size変化)でキャッシュ無効化される
        fs::write(&src, "hello world, longer content").unwrap();
        assert!(cache.get(&src).unwrap().is_none());
    }

    #[test]
    fn clear_removes_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        let cache = Cache::with_dir(cache_dir).unwrap();
        let src = tmp.path().join("source.jsonl");
        fs::write(&src, "hello").unwrap();
        cache.put(&src, vec![sample_session("s1")]).unwrap();
        assert_eq!(cache.status().unwrap().0, 1);
        cache.clear().unwrap();
        assert_eq!(cache.status().unwrap().0, 0);
    }
}
