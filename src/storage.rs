use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionKey {
    pub sender_comp_id: String,
    pub target_comp_id: String,
}

impl SessionKey {
    pub fn file_stem(&self) -> String {
        format!("{}__{}", sanitize(&self.sender_comp_id), sanitize(&self.target_comp_id))
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Direction { Inbound, Outbound }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessageRecord {
    pub session: SessionKey,
    pub direction: Direction,
    pub seq: Option<u32>,
    pub ts_millis: u64,
    pub payload_b64: String,
}

#[async_trait]
pub trait MessageStore: Send + Sync + 'static {
    async fn append(&self, record: StoredMessageRecord) -> std::io::Result<()>;
    async fn load_outbound_range(&self, session: &SessionKey, begin_seq: u32, end_seq: u32) -> std::io::Result<Vec<Bytes>>;
    async fn last_outbound_seq(&self, session: &SessionKey) -> std::io::Result<Option<u32>>;
}

#[derive(Clone)]
pub struct FileMessageStore {
    base_dir: PathBuf,
}

impl FileMessageStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self { base_dir: base_dir.into() }
    }

    fn file_path(&self, session: &SessionKey) -> PathBuf {
        self.base_dir.join(format!("{}.jsonl", session.file_stem()))
    }
}

#[async_trait]
impl MessageStore for FileMessageStore {
    async fn append(&self, record: StoredMessageRecord) -> std::io::Result<()> {
        fs::create_dir_all(&self.base_dir).await.ok();
        let path = self.file_path(&record.session);
        let mut f = OpenOptions::new().create(true).append(true).open(path).await?;
        let line = serde_json::to_string(&record).unwrap();
        f.write_all(line.as_bytes()).await?;
        f.write_all(b"\n").await?;
        Ok(())
    }

    async fn load_outbound_range(&self, session: &SessionKey, begin_seq: u32, end_seq: u32) -> std::io::Result<Vec<Bytes>> {
        let path = self.file_path(session);
        let mut out: Vec<(u32, Bytes)> = Vec::new();
        let content = match fs::read_to_string(&path).await { Ok(s) => s, Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound { return Ok(Vec::new()); }
            else { return Err(e); }
        }};
        for line in content.lines() {
            if line.trim().is_empty() { continue; }
            if let Ok(rec) = serde_json::from_str::<StoredMessageRecord>(line) {
                if rec.direction == Direction::Outbound {
                    if let Some(seq) = rec.seq {
                        if seq >= begin_seq && seq <= end_seq {
                            if let Ok(bytes) = base64::decode(&rec.payload_b64) {
                                out.push((seq, Bytes::from(bytes)));
                            }
                        }
                    }
                }
            }
        }
        out.sort_by_key(|(s, _)| *s);
        Ok(out.into_iter().map(|(_, b)| b).collect())
    }

    async fn last_outbound_seq(&self, session: &SessionKey) -> std::io::Result<Option<u32>> {
        let path = self.file_path(session);
        let content = match fs::read_to_string(&path).await { Ok(s) => s, Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound { return Ok(None); }
            else { return Err(e); }
        }};
        let mut max_seq: Option<u32> = None;
        for line in content.lines() {
            if let Ok(rec) = serde_json::from_str::<StoredMessageRecord>(line) {
                if rec.direction == Direction::Outbound {
                    if let Some(seq) = rec.seq { max_seq = Some(max_seq.map_or(seq, |m| m.max(seq))); }
                }
            }
        }
        Ok(max_seq)
    }
}