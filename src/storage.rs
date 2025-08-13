use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{PathBuf};
use std::sync::Arc;
use tokio::fs::{self, OpenOptions, File, metadata};
use tokio::io::{AsyncWriteExt, AsyncSeekExt, AsyncReadExt, BufReader, AsyncBufReadExt};
use tokio::sync::mpsc;
use tokio::time::{self, Duration, Instant};

use crate::config::StorageBackend;

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

#[derive(Debug, Clone)]
pub enum DurabilityPolicy {
    Always,
    IntervalMs(u64),
    Disabled,
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub base_dir: PathBuf,
    pub channel_capacity: usize,
    pub batch_max: usize,
    pub flush_interval_ms: u64,
    pub durability: DurabilityPolicy,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            base_dir: PathBuf::from("data/journal"),
            channel_capacity: 8192,
            batch_max: 1024,
            flush_interval_ms: 50,
            durability: DurabilityPolicy::IntervalMs(500),
        }
    }
}

#[async_trait]
pub trait MessageStore: Send + Sync + 'static {
    async fn append(&self, record: StoredMessageRecord) -> std::io::Result<()>;
    async fn load_outbound_range(&self, session: &SessionKey, begin_seq: u32, end_seq: u32) -> std::io::Result<Vec<Bytes>>;
    async fn last_outbound_seq(&self, session: &SessionKey) -> std::io::Result<Option<u32>>;
}

#[cfg(feature = "aeron-ffi")]
use crate::aeron_ffi::{AeronClient, Publication};

#[cfg(feature = "aeron-ffi")]
pub struct AeronMessageStore {
    client: AeronClient,
    publication: Publication,
}

#[cfg(feature = "aeron-ffi")]
impl AeronMessageStore {
    pub fn new_with_params(channel: &str, stream_id: i32) -> std::io::Result<Self> {
        let client = AeronClient::connect()?;
        let publication = Publication::add(&client, channel, stream_id)?;
        Ok(Self { client, publication })
    }
    pub fn new() -> Self { panic!("use new_with_params") }
}

#[cfg(feature = "aeron-ffi")]
#[async_trait]
impl MessageStore for AeronMessageStore {
    async fn append(&self, record: StoredMessageRecord) -> std::io::Result<()> {
        // Serialize to wire bytes; in real impl, offer raw FIX bytes not JSON.
        let bytes = serde_json::to_vec(&record).unwrap();
        let _ = self.publication.offer(&bytes)?;
        Ok(())
    }
    async fn load_outbound_range(&self, _session: &SessionKey, _begin_seq: u32, _end_seq: u32) -> std::io::Result<Vec<Bytes>> {
        // TODO: Implement archive replay to a subscription and collect buffers
        Err(std::io::Error::new(std::io::ErrorKind::Other, "Aeron replay not implemented"))
    }
    async fn last_outbound_seq(&self, _session: &SessionKey) -> std::io::Result<Option<u32>> { Ok(None) }
}

pub fn make_store(backend: &StorageBackend) -> Arc<dyn MessageStore> {
    match backend {
        StorageBackend::File { base_dir } => Arc::new(FileMessageStore::new(base_dir.clone())),
        StorageBackend::Aeron { archive_channel, stream_id } => {
            #[cfg(feature = "aeron-ffi")] {
                Arc::new(AeronMessageStore::new_with_params(archive_channel, *stream_id).expect("AeronMessageStore init"))
            }
            #[cfg(not(feature = "aeron-ffi"))] {
                Arc::new(FileMessageStore::new("data/journal"))
            }
        }
    }
}

#[derive(Clone)]
pub struct FileMessageStore {
    tx: mpsc::Sender<StoredMessageRecord>,
    cfg: StorageConfig,
}

impl FileMessageStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self::new_with_config(StorageConfig { base_dir: base_dir.into(), ..StorageConfig::default() })
    }

    pub fn new_with_config(cfg: StorageConfig) -> Self {
        let (tx, mut rx) = mpsc::channel::<StoredMessageRecord>(cfg.channel_capacity);
        let cfg_clone = cfg.clone();
        tokio::spawn(async move {
            let _ = fs::create_dir_all(&cfg_clone.base_dir).await;
            let mut queue: VecDeque<StoredMessageRecord> = VecDeque::with_capacity(cfg_clone.batch_max);
            let mut ticker = time::interval(Duration::from_millis(cfg_clone.flush_interval_ms));
            let mut last_sync: Instant = Instant::now();

            loop {
                tokio::select! {
                    maybe = rx.recv() => {
                        match maybe {
                            Some(rec) => { queue.push_back(rec); },
                            None => { flush_batch(&cfg_clone, &mut queue, &mut last_sync).await.ok(); break; }
                        }
                        if queue.len() >= cfg_clone.batch_max { let _ = flush_batch(&cfg_clone, &mut queue, &mut last_sync).await; }
                    }
                    _ = ticker.tick() => {
                        if !queue.is_empty() { let _ = flush_batch(&cfg_clone, &mut queue, &mut last_sync).await; }
                    }
                }
            }
        });
        Self { tx, cfg }
    }
}

async fn flush_batch(cfg: &StorageConfig, queue: &mut VecDeque<StoredMessageRecord>, last_sync: &mut Instant) -> std::io::Result<()> {
    while let Some(rec) = queue.pop_front() {
        let stem = rec.session.file_stem();
        let data_path = cfg.base_dir.join(format!("{}.jsonl", stem));
        let idx_path = cfg.base_dir.join(format!("{}.idx", stem));

        // Compute current offset before writing
        let offset = match metadata(&data_path).await { Ok(m) => m.len(), Err(_) => 0 };

        let mut f = OpenOptions::new().create(true).append(true).open(&data_path).await?;
        let line = serde_json::to_string(&rec).unwrap();
        f.write_all(line.as_bytes()).await?;
        f.write_all(b"\n").await?;

        if let Direction::Outbound = rec.direction {
            if let Some(seq) = rec.seq {
                let mut idx = OpenOptions::new().create(true).append(true).open(&idx_path).await?;
                let idx_line = format!("{} {}\n", seq, offset);
                idx.write_all(idx_line.as_bytes()).await?;
            }
        }

        match cfg.durability {
            DurabilityPolicy::Always => { let _ = f.sync_data().await; }
            DurabilityPolicy::IntervalMs(ms) => {
                if last_sync.elapsed() >= Duration::from_millis(ms) { let _ = f.sync_data().await; *last_sync = Instant::now(); }
            }
            DurabilityPolicy::Disabled => {}
        }
    }
    Ok(())
}

#[async_trait]
impl MessageStore for FileMessageStore {
    async fn append(&self, record: StoredMessageRecord) -> std::io::Result<()> {
        self.tx.send(record).await.map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "storage channel closed"))
    }

    async fn load_outbound_range(&self, session: &SessionKey, begin_seq: u32, end_seq: u32) -> std::io::Result<Vec<Bytes>> {
        let stem = session.file_stem();
        let data_path = self.cfg.base_dir.join(format!("{}.jsonl", stem));
        let idx_path = self.cfg.base_dir.join(format!("{}.idx", stem));

        // Read index and collect offsets
        let idx_content = match fs::read_to_string(&idx_path).await { Ok(s) => s, Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound { return Ok(Vec::new()); } else { return Err(e); }
        }};
        let mut offsets: Vec<(u32, u64)> = Vec::new();
        for line in idx_content.lines() {
            let mut it = line.split_whitespace();
            let seq = it.next().and_then(|s| s.parse::<u32>().ok());
            let off = it.next().and_then(|s| s.parse::<u64>().ok());
            if let (Some(sq), Some(of)) = (seq, off) {
                if sq >= begin_seq && sq <= end_seq { offsets.push((sq, of)); }
            }
        }
        offsets.sort_by_key(|(s, _)| *s);

        // Open data file once, then seek to read each record line
        let mut file = File::open(&data_path).await?;
        let mut out: Vec<Bytes> = Vec::with_capacity(offsets.len());
        for (_seq, of) in offsets {
            file.seek(std::io::SeekFrom::Start(of)).await?;
            let mut reader = BufReader::new(&mut file);
            let mut line = String::new();
            reader.read_line(&mut line).await?;
            if line.trim().is_empty() { continue; }
            if let Ok(rec) = serde_json::from_str::<StoredMessageRecord>(&line) {
                if let Ok(bytes) = base64::decode(&rec.payload_b64) {
                    out.push(Bytes::from(bytes));
                }
            }
        }
        Ok(out)
    }

    async fn last_outbound_seq(&self, session: &SessionKey) -> std::io::Result<Option<u32>> {
        let stem = session.file_stem();
        let idx_path = self.cfg.base_dir.join(format!("{}.idx", stem));
        let content = match fs::read_to_string(&idx_path).await { Ok(s) => s, Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound { return Ok(None); } else { return Err(e); }
        }};
        let mut last: Option<u32> = None;
        for line in content.lines() {
            let seq = line.split_whitespace().next().and_then(|s| s.parse::<u32>().ok());
            if let Some(sq) = seq { last = Some(last.map_or(sq, |m| m.max(sq))); }
        }
        Ok(last)
    }
}

#[cfg(feature = "aeron-ffi")]
#[link(name = "aeron")]
extern "C" {}