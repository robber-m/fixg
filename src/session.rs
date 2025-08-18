use crate::error::{FixgError, Result};
use crate::messages::AdminMessage;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

/// Reasons why a FIX session might be disconnected.
///
/// This enum categorizes the different conditions that can lead
/// to a session termination for proper error handling and logging.
#[derive(Debug, Clone, Copy)]
pub enum DisconnectReason {
    /// The remote peer closed the connection
    PeerClosed,
    /// A FIX protocol error occurred
    ProtocolError,
    /// Connection timed out due to inactivity
    Timeout,
    /// The application requested disconnection
    ApplicationRequested,
    /// Disconnect reason is unknown or unspecified
    Unknown,
}

/// Types of outbound messages that can be sent through a FIX session.
///
/// This enum distinguishes between raw byte payloads and structured
/// administrative messages for proper handling and routing.
#[derive(Debug, Clone)]
pub enum OutboundPayload {
    /// Raw bytes to be sent as-is
    Raw(Bytes),
    /// Structured administrative message (logon, heartbeat, etc.)
    Admin(AdminMessage),
}

/// Represents an active FIX session.
///
/// A session provides the interface for sending messages and managing
/// the connection state between two FIX endpoints. Each session has a
/// unique identifier and maintains its own message sending channel.
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique identifier for this session
    id: u64,
    /// Channel for sending outbound messages
    send_tx: mpsc::Sender<OutboundPayload>,
}

impl Session {
    pub fn id(&self) -> u64 {
        self.id
    }

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

/// Configuration for establishing a FIX session.
///
/// Contains all the necessary parameters to initiate a connection
/// and establish a FIX session with a counterparty.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Hostname or IP address of the target system
    pub host: String,
    /// Port number for the connection
    pub port: u16,
    /// FIX SenderCompID - identifies this system
    pub sender_comp_id: String,
    /// FIX TargetCompID - identifies the counterparty system
    pub target_comp_id: String,
    /// Heartbeat interval in seconds
    pub heartbeat_interval_secs: u32,
}

impl SessionConfig {
    pub fn builder() -> SessionConfigBuilder {
        SessionConfigBuilder::default()
    }
}

/// Builder pattern implementation for constructing SessionConfig instances.
///
/// Provides a fluent interface for setting session configuration parameters
/// with validation and default values.
#[derive(Debug, Default)]
pub struct SessionConfigBuilder {
    /// Target hostname or IP address
    host: Option<String>,
    /// Target port number
    port: Option<u16>,
    /// This system's FIX SenderCompID
    sender_comp_id: Option<String>,
    /// Counterparty's FIX TargetCompID
    target_comp_id: Option<String>,
    /// Heartbeat interval in seconds
    heartbeat_interval_secs: Option<u32>,
}

impl SessionConfigBuilder {
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }
    pub fn sender_comp_id(mut self, v: impl Into<String>) -> Self {
        self.sender_comp_id = Some(v.into());
        self
    }
    pub fn target_comp_id(mut self, v: impl Into<String>) -> Self {
        self.target_comp_id = Some(v.into());
        self
    }
    pub fn heartbeat_interval_secs(mut self, v: u32) -> Self {
        self.heartbeat_interval_secs = Some(v);
        self
    }

    pub fn build(self) -> Result<SessionConfig> {
        Ok(SessionConfig {
            host: self
                .host
                .ok_or_else(|| FixgError::InvalidConfig("host missing".into()))?,
            port: self
                .port
                .ok_or_else(|| FixgError::InvalidConfig("port missing".into()))?,
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
    (
        Session {
            id: session_id,
            send_tx: tx,
        },
        rx,
    )
}
