use crate::config::FixClientConfig;
use crate::error::{FixgError, Result};
use crate::gateway::{GatewayHandle, GatewayToClientEvent, GatewayClientCommand, GatewaySessionHandle};
use crate::session::{new_session, DisconnectReason, Session, SessionConfig};
use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
pub struct InboundMessage {
    msg_type: String,
    payload: Bytes,
}

impl InboundMessage {
    pub fn msg_type(&self) -> &str { &self.msg_type }
    pub fn body(&self) -> &Bytes { &self.payload }
}

#[async_trait]
pub trait FixHandler: Send {
    async fn on_message(&mut self, _session: &Session, _msg: InboundMessage) {}
    async fn on_session_active(&mut self, _session: &Session) {}
    async fn on_disconnect(&mut self, _session: &Session, _reason: DisconnectReason) {}
}

pub struct FixClient {
    library_id: i32,
    events_rx: mpsc::Receiver<GatewayToClientEvent>,
    cmd_tx: mpsc::Sender<GatewayClientCommand>,
    outbound_tx: mpsc::Sender<Bytes>,
    current_session: Option<Session>,
}

impl FixClient {
    pub async fn connect(config: FixClientConfig, handle: GatewayHandle) -> Result<Self> {
        let conn = handle.register_client(config.library_id).await?;
        // Create a per-session outbound channel that would be wired to gateway send path.
        let (outbound_tx, mut outbound_rx) = mpsc::channel::<Bytes>(1024);
        let cmd_tx = conn.cmd_tx.clone();

        // Wire outbound sends to gateway client command channel (no-op for now)
        tokio::spawn(async move {
            while let Some(payload) = outbound_rx.recv().await {
                // In a full implementation we would include session id and route to network.
                let _ = cmd_tx.send(GatewayClientCommand::Send { _session_id: 0, _payload: payload }).await;
            }
        });

        Ok(Self {
            library_id: config.library_id,
            events_rx: conn.events_rx,
            cmd_tx: conn.cmd_tx,
            outbound_tx,
            current_session: None,
        })
    }

    pub async fn initiate(&mut self, _cfg: SessionConfig) -> Result<Session> {
        // Ask gateway to create a session, then produce a Session handle and send SessionActive event.
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(GatewayClientCommand::InitiateSession { respond_to: tx })
            .await
            .map_err(|_| FixgError::ChannelClosed)?;
        let handle: GatewaySessionHandle = rx.await.map_err(|_| FixgError::ChannelClosed)?;

        let (session, _out_rx) = new_session(handle.session_id);
        self.current_session = Some(session.clone());
        Ok(session)
    }

    pub async fn run<H: FixHandler>(&mut self, handler: &mut H) -> Result<()> {
        while let Some(event) = self.events_rx.recv().await {
            match event {
                GatewayToClientEvent::SessionActive { session_id } => {
                    // Ensure we have a session handle; if not, create one
                    if self.current_session.is_none() {
                        let (session, _out_rx) = new_session(session_id);
                        self.current_session = Some(session);
                    }
                    if let Some(ref session) = self.current_session {
                        handler.on_session_active(session).await;
                    }
                }
                GatewayToClientEvent::InboundMessage { session_id: _, msg_type, payload } => {
                    if let Some(ref session) = self.current_session {
                        handler
                            .on_message(session, InboundMessage { msg_type, payload })
                            .await;
                    }
                }
                GatewayToClientEvent::Disconnected { session_id: _, reason } => {
                    if let Some(ref session) = self.current_session {
                        handler.on_disconnect(session, reason).await;
                    }
                }
            }
        }
        Err(FixgError::ChannelClosed)
    }
}

// Re-export config type for convenience
pub use crate::config::FixClientConfig as _FixClientConfigReExport;