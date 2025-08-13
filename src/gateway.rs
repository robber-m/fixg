use crate::config::GatewayConfig;
use crate::error::{FixgError, Result};
use crate::session::DisconnectReason;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone)]
pub struct GatewayHandle {
    cmd_tx: mpsc::Sender<GatewayCommand>,
}

impl GatewayHandle {
    pub async fn register_client(&self, library_id: i32) -> Result<ClientConnection> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(GatewayCommand::RegisterClient { library_id, respond_to: tx })
            .await
            .map_err(|_| FixgError::ChannelClosed)?;
        rx.await.map_err(|_| FixgError::ChannelClosed)
    }
}

pub struct Gateway;

impl Gateway {
    pub async fn spawn(_config: GatewayConfig) -> Result<GatewayHandle> {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<GatewayCommand>(1024);

        tokio::spawn(async move {
            // Minimal gateway loop: manages client registrations and routes simple messages.
            let mut next_session_id: u64 = 1;
            let mut clients: Vec<ClientConnectionInternal> = Vec::new();

            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    GatewayCommand::RegisterClient { library_id, respond_to } => {
                        let (to_client_tx, to_client_rx) = mpsc::channel::<GatewayEvent>(1024);
                        let (from_client_tx, mut from_client_rx) = mpsc::channel::<ClientCommand>(1024);

                        // Spawn a lightweight task to handle client-originated commands (e.g., initiate session)
                        let to_client_tx_clone = to_client_tx.clone();
                        tokio::spawn(async move {
                            while let Some(cc) = from_client_rx.recv().await {
                                match cc {
                                    ClientCommand::InitiateSession { respond_to } => {
                                        let session_id = next_session_id;
                                        // Send SessionActive event
                                        let _ = to_client_tx_clone
                                            .send(GatewayEvent::SessionActive { session_id })
                                            .await;
                                        let _ = respond_to.send(SessionHandle { session_id });
                                    }
                                    ClientCommand::Send { _session_id, _payload } => {
                                        // In a real gateway, this would write to the network.
                                    }
                                }
                            }
                        });

                        let _ = respond_to.send(ClientConnection {
                            events_rx: to_client_rx,
                            cmd_tx: from_client_tx,
                            _library_id: library_id,
                        });

                        clients.push(ClientConnectionInternal { _library_id: library_id, to_client_tx });
                    }
                    GatewayCommand::Shutdown => {
                        break;
                    }
                }
            }
        });

        Ok(GatewayHandle { cmd_tx })
    }
}

#[derive(Debug)]
pub struct ClientConnection {
    pub(crate) events_rx: mpsc::Receiver<GatewayEvent>,
    pub(crate) cmd_tx: mpsc::Sender<ClientCommand>,
    pub(crate) _library_id: i32,
}

struct ClientConnectionInternal {
    _library_id: i32,
    to_client_tx: mpsc::Sender<GatewayEvent>,
}

#[derive(Debug)]
pub enum GatewayEvent {
    SessionActive { session_id: u64 },
    InboundMessage { session_id: u64, msg_type: String, payload: Bytes },
    Disconnected { session_id: u64, reason: DisconnectReason },
}

#[derive(Debug)]
pub enum GatewayCommand {
    RegisterClient { library_id: i32, respond_to: oneshot::Sender<ClientConnection> },
    Shutdown,
}

#[derive(Debug)]
pub enum ClientCommand {
    InitiateSession { respond_to: oneshot::Sender<SessionHandle> },
    Send { _session_id: u64, _payload: Bytes },
}

#[derive(Debug, Clone, Copy)]
pub struct SessionHandle {
    pub session_id: u64,
}

// Re-export for client module
pub(crate) use ClientCommand as GatewayClientCommand;
pub(crate) use GatewayEvent as GatewayToClientEvent;
pub(crate) use SessionHandle as GatewaySessionHandle;