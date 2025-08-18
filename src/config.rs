use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub log_directory: PathBuf,
    pub aeron_channel: String,
    pub bind_address: SocketAddr,
    pub async_runtime: AsyncRuntime,
    #[serde(skip)]
    pub auth_strategy: Arc<dyn AuthStrategy>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            log_directory: PathBuf::from("./fixg_logs/"),
            aeron_channel: "aeron:ipc".to_string(),
            bind_address: "0.0.0.0:4050".parse().unwrap(),
            async_runtime: AsyncRuntime::MultiThread,
            auth_strategy: Arc::new(AcceptAllAuth),
        }
    }
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

/// Strategy interface for authenticating inbound Logon messages in acceptor mode.
pub trait AuthStrategy: Send + Sync {
    fn validate_logon(&self, sender_comp_id: &str, target_comp_id: &str) -> bool;
}

/// Default permissive authentication: accepts all logons.
#[derive(Debug, Clone, Copy)]
pub struct AcceptAllAuth;

impl AuthStrategy for AcceptAllAuth {
    fn validate_logon(&self, _sender_comp_id: &str, _target_comp_id: &str) -> bool { true }
}