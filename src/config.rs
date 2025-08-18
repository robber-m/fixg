use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

/// Configuration settings for the FIX gateway.
/// 
/// This struct contains all the necessary configuration options to set up and run
/// a FIX gateway, including networking, storage, and authentication settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Directory where log files will be stored
    pub log_directory: PathBuf,
    /// Aeron messaging channel configuration string
    pub aeron_channel: String,
    /// Socket address where the gateway will bind and listen for connections
    pub bind_address: SocketAddr,
    /// Type of async runtime to use (single-threaded or multi-threaded)
    pub async_runtime: AsyncRuntime,
    /// Storage backend configuration for message persistence
    pub storage: StorageBackend,
    /// Authentication strategy for validating incoming connections
    #[serde(skip, default = "default_auth_strategy")]
    pub auth_strategy: Arc<dyn AuthStrategy>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            log_directory: PathBuf::from("./fixg_logs/"),
            aeron_channel: "aeron:ipc".to_string(),
            bind_address: "0.0.0.0:4050".parse().unwrap(),
            async_runtime: AsyncRuntime::MultiThread,
            storage: StorageBackend::File { base_dir: PathBuf::from("data/journal") },
            auth_strategy: Arc::new(AcceptAllAuth),
        }
    }
}

/// Storage backend options for message persistence.
/// 
/// Defines different storage mechanisms that can be used to persist
/// FIX messages for replay and audit purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageBackend {
    /// File-based storage using local filesystem
    File { 
        /// Base directory where message files will be stored
        base_dir: PathBuf 
    },
    /// Aeron-based storage using Aeron Archive
    Aeron { 
        /// Aeron channel string for the archive
        archive_channel: String, 
        /// Stream ID for the archive
        stream_id: i32 
    },
}

/// Configuration settings for a FIX client connection.
/// 
/// Contains the necessary settings to establish a client connection
/// to a FIX gateway and manage its runtime behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixClientConfig {
    /// Unique identifier for this client library instance
    pub library_id: i32,
    /// Type of async runtime to use for this client
    pub async_runtime: AsyncRuntime,
}

impl FixClientConfig {
    pub fn new(library_id: i32) -> Self {
        Self { library_id, async_runtime: AsyncRuntime::MultiThread }
    }
}

/// Async runtime configuration options.
/// 
/// Specifies the type of Tokio runtime to use for async operations.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AsyncRuntime {
    /// Single-threaded runtime, all tasks run on the current thread
    CurrentThread,
    /// Multi-threaded runtime, tasks can be distributed across multiple threads
    MultiThread,
}

/// Strategy interface for authenticating inbound Logon messages in acceptor mode.
pub trait AuthStrategy: Send + Sync + std::fmt::Debug {
    fn validate_logon(&self, sender_comp_id: &str, target_comp_id: &str) -> bool;
}

/// Default permissive authentication strategy that accepts all logons.
/// 
/// This is a simple authentication implementation that allows all
/// incoming logon requests without any validation. Useful for
/// development and testing environments.
#[derive(Debug, Clone, Copy)]
pub struct AcceptAllAuth;

impl AuthStrategy for AcceptAllAuth {
    fn validate_logon(&self, _sender_comp_id: &str, _target_comp_id: &str) -> bool { true }
}

fn default_auth_strategy() -> Arc<dyn AuthStrategy> { Arc::new(AcceptAllAuth) }