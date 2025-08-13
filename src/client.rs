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
    current_session: Option<Session>,
}

impl FixClient {
    pub async fn connect(config: FixClientConfig, handle: GatewayHandle) -> Result<Self> {
        let conn = handle.register_client(config.library_id).await?;
        Ok(Self {
            library_id: config.library_id,
            events_rx: conn.events_rx,
            cmd_tx: conn.cmd_tx,
            current_session: None,
        })
    }

    pub async fn initiate(&mut self, cfg: SessionConfig) -> Result<Session> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(GatewayClientCommand::InitiateSession {
                host: cfg.host.clone(),
                port: cfg.port,
                sender_comp_id: cfg.sender_comp_id.clone(),
                target_comp_id: cfg.target_comp_id.clone(),
                heartbeat_interval_secs: cfg.heartbeat_interval_secs,
                respond_to: tx,
            })
            .await
            .map_err(|_| FixgError::ChannelClosed)?;
        let handle: GatewaySessionHandle = rx.await.map_err(|_| FixgError::ChannelClosed)?;

        let session_id = handle.session_id;
        let cmd_tx = self.cmd_tx.clone();
        let (session, mut out_rx) = new_session(session_id);

        // Route outbound payloads to gateway with session id
        tokio::spawn(async move {
            while let Some(payload) = out_rx.recv().await {
                let _ = cmd_tx
                    .send(GatewayClientCommand::Send { session_id, payload })
                    .await;
            }
        });

        self.current_session = Some(session.clone());
        Ok(session)
    }

    pub async fn run<H: FixHandler>(&mut self, handler: &mut H) -> Result<()> {
        while let Some(event) = self.events_rx.recv().await {
            match event {
                GatewayToClientEvent::SessionActive { session_id } => {
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