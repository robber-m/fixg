use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub log_directory: PathBuf,
    pub aeron_channel: String,
    pub bind_address: SocketAddr,
    pub async_runtime: AsyncRuntime,
    pub storage: StorageBackend,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            log_directory: PathBuf::from("./fixg_logs/"),
            aeron_channel: "aeron:ipc".to_string(),
            bind_address: "0.0.0.0:4050".parse().unwrap(),
            async_runtime: AsyncRuntime::MultiThread,
            storage: StorageBackend::File { base_dir: PathBuf::from("data/journal") },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageBackend {
    File { base_dir: PathBuf },
    Aeron { archive_channel: String, stream_id: i32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixClientConfig {
    pub library_id: i32,
    pub async_runtime: AsyncRuntime,
}

impl FixClientConfig {
    pub fn new(library_id: i32) -> Self {
        Self { library_id, async_runtime: AsyncRuntime::MultiThread }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AsyncRuntime {
    CurrentThread,
    MultiThread,
}