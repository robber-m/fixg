use crate::error::{FixgError, Result};
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use crate::messages::AdminMessage;

#[derive(Debug, Clone, Copy)]
pub enum DisconnectReason {
    PeerClosed,
    ProtocolError,
    Timeout,
    ApplicationRequested,
    Unknown,
}

#[derive(Debug, Clone)]
pub enum OutboundPayload {
    Raw(Bytes),
    Admin(AdminMessage),
}

#[derive(Debug, Clone)]
pub struct Session {
    id: u64,
    send_tx: mpsc::Sender<OutboundPayload>,
}

impl Session {
    pub fn id(&self) -> u64 { self.id }

    pub async fn send(&self, payload: Bytes) -> Result<()> {
        self.send_tx
            .send(OutboundPayload::Raw(payload))
            .await
            .map_err(|_| FixgError::ChannelClosed)
            .map(|_| ())
    }

    pub async fn send_admin(&self, msg: AdminMessage) -> Result<()> {
        self.send_tx
            .send(OutboundPayload::Admin(msg))
            .await
            .map_err(|_| FixgError::ChannelClosed)
            .map(|_| ())
    }
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub host: String,
    pub port: u16,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub heartbeat_interval_secs: u32,
}

impl SessionConfig {
    pub fn builder() -> SessionConfigBuilder { SessionConfigBuilder::default() }
}

#[derive(Debug, Default)]
pub struct SessionConfigBuilder {
    host: Option<String>,
    port: Option<u16>,
    sender_comp_id: Option<String>,
    target_comp_id: Option<String>,
    heartbeat_interval_secs: Option<u32>,
}

impl SessionConfigBuilder {
    pub fn host(mut self, host: impl Into<String>) -> Self { self.host = Some(host.into()); self }
    pub fn port(mut self, port: u16) -> Self { self.port = Some(port); self }
    pub fn sender_comp_id(mut self, v: impl Into<String>) -> Self { self.sender_comp_id = Some(v.into()); self }
    pub fn target_comp_id(mut self, v: impl Into<String>) -> Self { self.target_comp_id = Some(v.into()); self }
    pub fn heartbeat_interval_secs(mut self, v: u32) -> Self { self.heartbeat_interval_secs = Some(v); self }

    pub fn build(self) -> Result<SessionConfig> {
        Ok(SessionConfig {
            host: self.host.ok_or_else(|| FixgError::InvalidConfig("host missing".into()))?,
            port: self.port.ok_or_else(|| FixgError::InvalidConfig("port missing".into()))?,
            sender_comp_id: self
                .sender_comp_id
                .ok_or_else(|| FixgError::InvalidConfig("sender_comp_id missing".into()))?,
            target_comp_id: self
                .target_comp_id
                .ok_or_else(|| FixgError::InvalidConfig("target_comp_id missing".into()))?,
            heartbeat_interval_secs: self.heartbeat_interval_secs.unwrap_or(30),
        })
    }
}

// Internal helper to create a Session with a send channel
pub(crate) fn new_session(session_id: u64) -> (Session, mpsc::Receiver<OutboundPayload>) {
    let (tx, rx) = mpsc::channel::<OutboundPayload>(1024);
    (Session { id: session_id, send_tx: tx }, rx)
}