use crate::config::GatewayConfig;
use crate::error::{FixgError, Result};
use crate::session::DisconnectReason;
use bytes::{Bytes, BytesMut};
use std::collections::HashMap;
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{self, Duration, Instant};

use crate::protocol::{self, FixMsgType};
use crate::messages::AdminMessage;
use crate::session::OutboundPayload;

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
        let next_session_id = Arc::new(AtomicU64::new(0));

        tokio::spawn({
            let next_session_id = Arc::clone(&next_session_id);
            async move {
                let mut _clients: Vec<ClientConnectionInternal> = Vec::new();

                while let Some(cmd) = cmd_rx.recv().await {
                    match cmd {
                        GatewayCommand::RegisterClient { library_id, respond_to } => {
                            let (to_client_tx, to_client_rx) = mpsc::channel::<GatewayEvent>(1024);
                            let (from_client_tx, mut from_client_rx) = mpsc::channel::<ClientCommand>(1024);

                            // Per-client task managing sessions and I/O
                            let to_client_tx_clone = to_client_tx.clone();
                            let next_id = Arc::clone(&next_session_id);
                            tokio::spawn(async move {
                                let mut session_senders: HashMap<u64, mpsc::Sender<OutboundPayload>> = HashMap::new();

                                while let Some(cc) = from_client_rx.recv().await {
                                    match cc {
                                        ClientCommand::InitiateSession { host, port, sender_comp_id, target_comp_id, heartbeat_interval_secs, respond_to } => {
                                            let addr = format!("{}:{}", host, port);
                                            match TcpStream::connect(addr).await {
                                                Ok(stream) => {
                                                    let session_id = next_id.fetch_add(1, Ordering::Relaxed) + 1;
                                                    let (mut read_half, write_half) = stream.into_split();

                                                    // Create channel for application-driven outbound payloads to this session task
                                                    let (app_out_tx, mut app_out_rx) = mpsc::channel::<OutboundPayload>(1024);
                                                    session_senders.insert(session_id, app_out_tx.clone());

                                                    // Spawn session task owning write half, performing handshake, timers, and parsing
                                                    let to_client_tx_reader = to_client_tx_clone.clone();
                                                    tokio::spawn(async move {
                                                        let mut write_half = write_half;
                                                        let hb_interval = Duration::from_secs(heartbeat_interval_secs as u64);
                                                        let mut out_seq_num: u32 = 1;
                                                        let mut in_seq_num: u32 = 0;
                                                        let mut last_rx: Instant = Instant::now();
                                                        let mut test_req_outstanding: Option<String> = None;

                                                        // Send Logon
                                                        let mut logon = protocol::build_logon(heartbeat_interval_secs, &sender_comp_id, &target_comp_id);
                                                        logon.set_field(34, out_seq_num.to_string());
                                                        out_seq_num += 1;
                                                        let logon_bytes = protocol::encode(logon);
                                                        let _ = write_half.write_all(&logon_bytes).await;

                                                        // Timers
                                                        let mut interval = time::interval(hb_interval);
                                                        interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

                                                        let mut read_buf = BytesMut::with_capacity(16 * 1024);

                                                        loop {
                                                            tokio::select! {
                                                                biased;
                                                                // Application outbound payloads
                                                                maybe_out = app_out_rx.recv() => {
                                                                    if let Some(payload) = maybe_out {
                                                                        match payload {
                                                                            OutboundPayload::Raw(bytes) => {
                                                                                let _ = write_half.write_all(&bytes).await;
                                                                            }
                                                                            OutboundPayload::Admin(msg) => {
                                                                                let mut fix = msg.into_fix(&sender_comp_id, &target_comp_id);
                                                                                fix.set_field(34, out_seq_num.to_string());
                                                                                out_seq_num += 1;
                                                                                let bytes = protocol::encode(fix);
                                                                                let _ = write_half.write_all(&bytes).await;
                                                                            }
                                                                        }
                                                                    } else {
                                                                        break;
                                                                    }
                                                                }
                                                                // Network reads
                                                                res = read_half.read_buf(&mut read_buf) => {
                                                                    match res {
                                                                        Ok(0) => {
                                                                            let _ = to_client_tx_reader
                                                                                .send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::PeerClosed })
                                                                                .await;
                                                                            break;
                                                                        }
                                                                        Ok(_n) => {
                                                                            // Try extract full messages
                                                                            while let Some(msg_bytes) = protocol::try_extract_one(&mut read_buf) {
                                                                                last_rx = Instant::now();
                                                                                match protocol::decode(&msg_bytes) {
                                                                                    Ok(mut msg) => {
                                                                                        // Check seqnum if present
                                                                                        if let Some(seq) = msg.fields.get(&34) {
                                                                                            if let Ok(seq_val) = seq.parse::<u32>() { in_seq_num = seq_val; }
                                                                                        }
                                                                                        match msg.msg_type {
                                                                                            FixMsgType::Logon => {
                                                                                                let _ = to_client_tx_reader
                                                                                                    .send(GatewayEvent::SessionActive { session_id })
                                                                                                    .await;
                                                                                            }
                                                                                            FixMsgType::Heartbeat => {
                                                                                                if let Some(id) = msg.fields.get(&112) {
                                                                                                    if test_req_outstanding.as_deref() == Some(id) {
                                                                                                        test_req_outstanding = None;
                                                                                                    }
                                                                                                }
                                                                                            }
                                                                                            FixMsgType::TestRequest => {
                                                                                                let tr_id = msg.fields.get(&112).cloned();
                                                                                                let mut hb = protocol::build_heartbeat(tr_id.as_deref(), &sender_comp_id, &target_comp_id);
                                                                                                hb.set_field(34, out_seq_num.to_string());
                                                                                                out_seq_num += 1;
                                                                                                let hb_bytes = protocol::encode(hb);
                                                                                                let _ = write_half.write_all(&hb_bytes).await;
                                                                                            }
                                                                                            FixMsgType::Logout => {
                                                                                                // Echo logout and close
                                                                                                let mut lo = protocol::build_logout(None, &sender_comp_id, &target_comp_id);
                                                                                                lo.set_field(34, out_seq_num.to_string());
                                                                                                out_seq_num += 1;
                                                                                                let lo_bytes = protocol::encode(lo);
                                                                                                let _ = write_half.write_all(&lo_bytes).await;
                                                                                                let _ = to_client_tx_reader
                                                                                                    .send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::ApplicationRequested })
                                                                                                    .await;
                                                                                                break;
                                                                                            }
                                                                                            FixMsgType::Unknown(_) => {}
                                                                                        }
                                                                                        // Forward inbound to client as event
                                                                                        let msg_type = match msg.msg_type { FixMsgType::Unknown(_) => "?".to_string(), _ => protocol::msg_type_as_str(&msg.msg_type).to_string() };
                                                                                        let _ = to_client_tx_reader
                                                                                            .send(GatewayEvent::InboundMessage { session_id, msg_type, payload: msg_bytes.clone() })
                                                                                            .await;
                                                                                    }
                                                                                    Err(_) => {
                                                                                        let _ = to_client_tx_reader
                                                                                            .send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::ProtocolError })
                                                                                            .await;
                                                                                        break;
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                        Err(_) => {
                                                                            let _ = to_client_tx_reader
                                                                                .send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::Unknown })
                                                                                .await;
                                                                            break;
                                                                        }
                                                                    }
                                                                }
                                                                // Heartbeat timers
                                                                _ = interval.tick() => {
                                                                    let idle = last_rx.elapsed();
                                                                    if idle >= hb_interval * 3 {
                                                                        let _ = to_client_tx_reader
                                                                            .send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::Timeout })
                                                                            .await;
                                                                        break;
                                                                    } else if idle >= hb_interval * 2 {
                                                                        if test_req_outstanding.is_none() {
                                                                            let tr_id = format!("TR-{}", out_seq_num);
                                                                            let mut tr = protocol::build_test_request(&tr_id, &sender_comp_id, &target_comp_id);
                                                                            tr.set_field(34, out_seq_num.to_string());
                                                                            out_seq_num += 1;
                                                                            let tr_bytes = protocol::encode(tr);
                                                                            let _ = write_half.write_all(&tr_bytes).await;
                                                                            test_req_outstanding = Some(tr_id);
                                                                        }
                                                                    } else if idle >= hb_interval {
                                                                        let mut hb = protocol::build_heartbeat(None, &sender_comp_id, &target_comp_id);
                                                                        hb.set_field(34, out_seq_num.to_string());
                                                                        out_seq_num += 1;
                                                                        let hb_bytes = protocol::encode(hb);
                                                                        let _ = write_half.write_all(&hb_bytes).await;
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    });

                                                    let _ = to_client_tx_clone
                                                        .send(GatewayEvent::SessionActive { session_id })
                                                        .await;
                                                    let _ = respond_to.send(SessionHandle { session_id });
                                                }
                                                Err(e) => {
                                                    let _ = respond_to.send(SessionHandle { session_id: 0 });
                                                    let _ = to_client_tx_clone
                                                        .send(GatewayEvent::Disconnected { session_id: 0, reason: DisconnectReason::Unknown })
                                                        .await;
                                                    tracing::error!(error = %e, "Failed to connect session");
                                                }
                                            }
                                        }
                                        ClientCommand::Send { session_id, payload } => {
                                            if let Some(tx) = session_senders.get_mut(&session_id) {
                                                let _ = tx.send(OutboundPayload::Raw(payload)).await;
                                            }
                                        }
                                        ClientCommand::SendAdmin { session_id, msg, .. } => {
                                            if let Some(tx) = session_senders.get_mut(&session_id) {
                                                let _ = tx.send(OutboundPayload::Admin(msg)).await;
                                            }
                                        }
                                    }
                                }
                            });

                            let _ = respond_to.send(ClientConnection {
                                events_rx: to_client_rx,
                                cmd_tx: from_client_tx,
                                _library_id: library_id,
                            });

                            _clients.push(ClientConnectionInternal { _library_id: library_id, to_client_tx });
                        }
                        GatewayCommand::Shutdown => {
                            break;
                        }
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
    InitiateSession { host: String, port: u16, sender_comp_id: String, target_comp_id: String, heartbeat_interval_secs: u32, respond_to: oneshot::Sender<SessionHandle> },
    Send { session_id: u64, payload: Bytes },
    SendAdmin { session_id: u64, msg: AdminMessage, sender_comp_id: String, target_comp_id: String },
}

#[derive(Debug, Clone, Copy)]
pub struct SessionHandle {
    pub session_id: u64,
}

// Re-export for client module
pub(crate) use ClientCommand as GatewayClientCommand;
pub(crate) use GatewayEvent as GatewayToClientEvent;
pub(crate) use SessionHandle as GatewaySessionHandle;