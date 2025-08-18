use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::{self, metadata, File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::time::{self, Duration, Instant};

use crate::config::StorageBackend;

/// Unique identifier for a FIX session based on company IDs.
///
/// Used to distinguish between different sessions for storage and
/// retrieval purposes in the message store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionKey {
    /// Sender's company identifier
    pub sender_comp_id: String,
    /// Target company identifier
    pub target_comp_id: String,
}

impl SessionKey {
    pub fn file_stem(&self) -> String {
        format!(
            "{}__{}",
            sanitize(&self.sender_comp_id),
            sanitize(&self.target_comp_id)
        )
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Direction of message flow relative to this system.
///
/// Indicates whether a message was received from or sent to a counterparty.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Direction {
    /// Message received from counterparty
    Inbound,
    /// Message sent to counterparty
    Outbound,
}

/// A persisted message record with metadata for storage and retrieval.
///
/// Contains all necessary information to store and later retrieve
/// a FIX message, including session context and sequencing information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessageRecord {
    /// Session this message belongs to
    pub session: SessionKey,
    /// Direction of the message (inbound/outbound)
    pub direction: Direction,
    /// Sequence number of the message (if applicable)
    pub seq: Option<u32>,
    /// Timestamp when message was processed (milliseconds since epoch)
    pub ts_millis: u64,
    /// Base64-encoded message payload
    pub payload_b64: String,
}

/// Policy for when to sync data to persistent storage.
///
/// Controls the trade-off between data safety and performance
/// by determining when writes are flushed to disk.
#[derive(Debug, Clone)]
pub enum DurabilityPolicy {
    /// Sync to disk after every write (safest, slowest)
    Always,
    /// Sync to disk at specified intervals in milliseconds
    IntervalMs(u64),
    /// Never explicitly sync (fastest, least safe)
    Disabled,
}

/// Configuration settings for the message storage system.
///
/// Controls various aspects of message persistence including
/// performance tuning and durability guarantees.
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Base directory for storing message files
    pub base_dir: PathBuf,
    /// Channel capacity for buffering messages before writing
    pub channel_capacity: usize,
    /// Maximum number of messages to batch before flushing
    pub batch_max: usize,
    /// How often to flush pending writes (milliseconds)
    pub flush_interval_ms: u64,
    /// Policy for syncing data to persistent storage
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
    async fn append_bytes(
        &self,
        session: &SessionKey,
        direction: Direction,
        seq: Option<u32>,
        ts_millis: u64,
        payload: &[u8],
    ) -> std::io::Result<()>;
    async fn load_outbound_range(
        &self,
        session: &SessionKey,
        begin_seq: u32,
        end_seq: u32,
    ) -> std::io::Result<Vec<Bytes>>;
    async fn last_outbound_seq(&self, session: &SessionKey) -> std::io::Result<Option<u32>>;
}

#[cfg(feature = "aeron-ffi")]
use crate::aeron_ffi::{AeronClient, Publication, Subscription};

#[cfg(feature = "aeron-ffi")]
pub struct AeronMessageStore {
    _client: AeronClient,
    data_pub: Publication,
    index_pub: Publication,
    index_sub: Subscription,
    _channel: String,
    _data_stream_id: i32,
    _index_stream_id: i32,
}

#[cfg(feature = "aeron-ffi")]
impl AeronMessageStore {
    pub fn new_with_params(channel: &str, stream_id: i32) -> std::io::Result<Self> {
        let client = AeronClient::connect()?;
        let data_pub = Publication::add(&client, channel, stream_id)?;
        let index_stream_id = stream_id + 1;
        let index_pub = Publication::add(&client, channel, index_stream_id)?;
        let index_sub = Subscription::add(&client, channel, index_stream_id)?;
        Ok(Self {
            _client: client,
            data_pub,
            index_pub,
            index_sub,
            _channel: channel.to_string(),
            _data_stream_id: stream_id,
            _index_stream_id: index_stream_id,
        })
    }
    pub fn new() -> Self {
        panic!("use new_with_params")
    }

    fn encode_index_frame(seq: u32, payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(4 + payload.len());
        v.extend_from_slice(&seq.to_be_bytes());
        v.extend_from_slice(payload);
        v
    }
}

#[cfg(feature = "aeron-ffi")]
#[async_trait]
impl MessageStore for AeronMessageStore {
    async fn append(&self, record: StoredMessageRecord) -> std::io::Result<()> {
        // Fallback to bytes path if possible
        if let Ok(bytes) = general_purpose::STANDARD.decode(&record.payload_b64) {
            self.append_bytes(
                &record.session,
                record.direction,
                record.seq,
                record.ts_millis,
                &bytes,
            )
            .await
        } else {
            Ok(())
        }
    }

    async fn append_bytes(
        &self,
        _session: &SessionKey,
        direction: Direction,
        seq: Option<u32>,
        _ts_millis: u64,
        payload: &[u8],
    ) -> std::io::Result<()> {
        if direction == Direction::Outbound {
            if let Some(s) = seq {
                let _ = self.data_pub.offer_retry(payload, 10, 100, 1)?;
                let idx = Self::encode_index_frame(s, payload);
                let _ = self.index_pub.offer_retry(&idx, 10, 100, 1)?;
            }
        }
        Ok(())
    }

    async fn load_outbound_range(
        &self,
        _session: &SessionKey,
        begin_seq: u32,
        end_seq: u32,
    ) -> std::io::Result<Vec<Bytes>> {
        let frags = self.index_sub.poll_collect(100, 25);
        let mut out: Vec<(u32, Bytes)> = Vec::new();
        for f in frags.into_iter() {
            if f.len() < 4 {
                continue;
            }
            let seq = u32::from_be_bytes([f[0], f[1], f[2], f[3]]);
            if seq >= begin_seq && seq <= end_seq {
                out.push((seq, Bytes::from(f[4..].to_vec())));
            }
        }
        out.sort_by_key(|(s, _)| *s);
        Ok(out.into_iter().map(|(_, b)| b).collect())
    }

    async fn last_outbound_seq(&self, _session: &SessionKey) -> std::io::Result<Option<u32>> {
        Ok(None)
    }
}

pub fn make_store(backend: &StorageBackend) -> Arc<dyn MessageStore> {
    match backend {
        StorageBackend::File { base_dir } => Arc::new(FileMessageStore::new(base_dir.clone())),
        StorageBackend::Aeron {
            archive_channel: _archive_channel,
            stream_id: _stream_id,
        } => {
            #[cfg(feature = "aeron-ffi")]
            {
                Arc::new(
                    AeronMessageStore::new_with_params(_archive_channel.as_str(), *_stream_id)
                        .expect("AeronMessageStore init"),
                )
            }
            #[cfg(not(feature = "aeron-ffi"))]
            {
                Arc::new(FileMessageStore::new("data/journal"))
            }
        }
    }
}

/// File-based implementation of message storage.
///
/// Stores FIX messages in JSON Lines format with separate index files
/// for efficient sequence number-based retrieval.
#[derive(Clone)]
pub struct FileMessageStore {
    /// Channel for sending messages to storage worker
    tx: mpsc::Sender<StoredMessageRecord>,
    /// Storage configuration settings
    cfg: StorageConfig,
}

impl FileMessageStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self::new_with_config(StorageConfig {
            base_dir: base_dir.into(),
            ..StorageConfig::default()
        })
    }

    pub fn new_with_config(cfg: StorageConfig) -> Self {
        let (tx, mut rx) = mpsc::channel::<StoredMessageRecord>(cfg.channel_capacity);
        let cfg_clone = cfg.clone();
        tokio::spawn(async move {
            let _ = fs::create_dir_all(&cfg_clone.base_dir).await;
            let mut queue: VecDeque<StoredMessageRecord> =
                VecDeque::with_capacity(cfg_clone.batch_max);
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

async fn flush_batch(
    cfg: &StorageConfig,
    queue: &mut VecDeque<StoredMessageRecord>,
    last_sync: &mut Instant,
) -> std::io::Result<()> {
    while let Some(rec) = queue.pop_front() {
        let stem = rec.session.file_stem();
        let data_path = cfg.base_dir.join(format!("{}.jsonl", stem));
        let idx_path = cfg.base_dir.join(format!("{}.idx", stem));

        // Compute current offset before writing
        let offset = match metadata(&data_path).await {
            Ok(m) => m.len(),
            Err(_) => 0,
        };

        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&data_path)
            .await?;
        let line = serde_json::to_string(&rec).unwrap();
        f.write_all(line.as_bytes()).await?;
        f.write_all(b"\n").await?;

        if let Direction::Outbound = rec.direction {
            if let Some(seq) = rec.seq {
                let mut idx = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&idx_path)
                    .await?;
                let idx_line = format!("{} {}\n", seq, offset);
                idx.write_all(idx_line.as_bytes()).await?;
            }
        }

        match cfg.durability {
            DurabilityPolicy::Always => {
                let _ = f.sync_data().await;
            }
            DurabilityPolicy::IntervalMs(ms) => {
                if last_sync.elapsed() >= Duration::from_millis(ms) {
                    let _ = f.sync_data().await;
                    *last_sync = Instant::now();
                }
            }
            DurabilityPolicy::Disabled => {}
        }
    }
    Ok(())
}

#[async_trait]
impl MessageStore for FileMessageStore {
    async fn append(&self, record: StoredMessageRecord) -> std::io::Result<()> {
        self.tx.send(record).await.map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "storage channel closed")
        })
    }

    async fn append_bytes(
        &self,
        session: &SessionKey,
        direction: Direction,
        seq: Option<u32>,
        ts_millis: u64,
        payload: &[u8],
    ) -> std::io::Result<()> {
        let rec = StoredMessageRecord {
            session: session.clone(),
            direction,
            seq,
            ts_millis,
            payload_b64: general_purpose::STANDARD.encode(payload),
        };
        self.append(rec).await
    }

    async fn load_outbound_range(
        &self,
        session: &SessionKey,
        begin_seq: u32,
        end_seq: u32,
    ) -> std::io::Result<Vec<Bytes>> {
        let stem = session.file_stem();
        let data_path = self.cfg.base_dir.join(format!("{}.jsonl", stem));
        let idx_path = self.cfg.base_dir.join(format!("{}.idx", stem));

        // Read index and collect offsets
        let idx_content = match fs::read_to_string(&idx_path).await {
            Ok(s) => s,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(Vec::new());
                } else {
                    return Err(e);
                }
            }
        };
        let mut offsets: Vec<(u32, u64)> = Vec::new();
        for line in idx_content.lines() {
            let mut it = line.split_whitespace();
            let seq = it.next().and_then(|s| s.parse::<u32>().ok());
            let off = it.next().and_then(|s| s.parse::<u64>().ok());
            if let (Some(sq), Some(of)) = (seq, off) {
                if sq >= begin_seq && sq <= end_seq {
                    offsets.push((sq, of));
                }
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
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(rec) = serde_json::from_str::<StoredMessageRecord>(&line) {
                if let Ok(bytes) = general_purpose::STANDARD.decode(&rec.payload_b64) {
                    out.push(Bytes::from(bytes));
                }
            }
        }
        Ok(out)
    }

    async fn last_outbound_seq(&self, session: &SessionKey) -> std::io::Result<Option<u32>> {
        let stem = session.file_stem();
        let idx_path = self.cfg.base_dir.join(format!("{}.idx", stem));
        let content = match fs::read_to_string(&idx_path).await {
            Ok(s) => s,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(None);
                } else {
                    return Err(e);
                }
            }
        };
        let mut last: Option<u32> = None;
        for line in content.lines() {
            let seq = line
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u32>().ok());
            if let Some(sq) = seq {
                last = Some(last.map_or(sq, |m| m.max(sq)));
            }
        }
        Ok(last)
    }
}

#[cfg(feature = "aeron-ffi")]
#[link(name = "aeron")]
extern "C" {}
