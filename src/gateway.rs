use crate::config::GatewayConfig;
use crate::error::{FixgError, Result};
use crate::session::DisconnectReason;
use bytes::{Bytes, BytesMut};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::{self, Duration, Instant};

use crate::messages::AdminMessage;
use crate::protocol::{self, FixMsgType};
use crate::session::OutboundPayload;
use crate::storage::{make_store, Direction, SessionKey};

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Handle for communicating with a running FIX gateway.
///
/// Provides a thread-safe interface for clients to register with
/// the gateway and send commands for session management.
#[derive(Debug, Clone)]
pub struct GatewayHandle {
    /// Channel for sending commands to the gateway
    cmd_tx: mpsc::Sender<GatewayCommand>,
}

impl GatewayHandle {
    pub async fn register_client(&self, library_id: i32) -> Result<ClientConnection> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(GatewayCommand::RegisterClient {
                library_id,
                respond_to: tx,
            })
            .await
            .map_err(|_| FixgError::ChannelClosed)?;
        rx.await.map_err(|_| FixgError::ChannelClosed)
    }
}

/// The main FIX gateway that manages sessions and client connections.
///
/// Acts as a central hub for all FIX communication, handling both
/// incoming acceptor connections and outgoing initiator sessions.
pub struct Gateway;

impl Gateway {
    pub async fn spawn(config: GatewayConfig) -> Result<GatewayHandle> {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<GatewayCommand>(1024);
        let next_session_id = Arc::new(AtomicU64::new(0));
        let global_session_senders: Arc<RwLock<HashMap<u64, mpsc::Sender<OutboundPayload>>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let clients: Arc<RwLock<Vec<mpsc::Sender<GatewayEvent>>>> =
            Arc::new(RwLock::new(Vec::new()));
        let store = make_store(&config.storage);

        tokio::spawn({
            let next_session_id = Arc::clone(&next_session_id);
            let global_session_senders = Arc::clone(&global_session_senders);
            let clients = Arc::clone(&clients);
            let bind_addr = config.bind_address;
            let auth = Arc::clone(&config.auth_strategy);
            let store = store.clone();
            async move {
                let mut _clients: Vec<ClientConnectionInternal> = Vec::new();

                // Start TCP listener for acceptor mode
                let listener = match TcpListener::bind(bind_addr).await {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to bind listener");
                        return;
                    }
                };

                // Accept loop in background
                tokio::spawn({
                    let next_id = Arc::clone(&next_session_id);
                    let clients = Arc::clone(&clients);
                    let global_session_senders = Arc::clone(&global_session_senders);
                    let auth = Arc::clone(&auth);
                    async move {
                        loop {
                            match listener.accept().await {
                                Ok((stream, _addr)) => {
                                    let session_id = next_id.fetch_add(1, Ordering::Relaxed) + 1;
                                    let (read_half, write_half) = stream.into_split();

                                    let (app_out_tx, app_out_rx) =
                                        mpsc::channel::<OutboundPayload>(1024);
                                    {
                                        let mut map = global_session_senders.write().await;
                                        map.insert(session_id, app_out_tx.clone());
                                    }

                                    tokio::spawn({
                                        let clients = Arc::clone(&clients);
                                        let auth = Arc::clone(&auth);
                                        async move {
                                            let mut write_half = write_half;
                                            let mut read_half = read_half;
                                            let mut app_out_rx = app_out_rx;
                                            let mut out_seq_num: u32 = 1;
                                            let mut _in_seq_num: u32 = 0;
                                            let mut last_rx: Instant = Instant::now();
                                            let mut test_req_outstanding: Option<String> = None;
                                            let mut hb_interval = Duration::from_secs(30);
                                            let mut sender_comp = String::new();
                                            let mut target_comp = String::new();
                                            let mut read_buf = BytesMut::with_capacity(16 * 1024);

                                            let mut tick = time::interval(Duration::from_secs(1));
                                            tick.set_missed_tick_behavior(
                                                time::MissedTickBehavior::Delay,
                                            );

                                            loop {
                                                tokio::select! {
                                                    biased;
                                                    maybe_out = app_out_rx.recv() => {
                                                        if let Some(payload) = maybe_out {
                                                            match payload {
                                                                OutboundPayload::Raw(bytes) => {
                                                                    let _ = write_half.write_all(&bytes).await;
                                                                }
                                                                OutboundPayload::Admin(msg) => {
                                                                    let mut fix = msg.into_fix(&target_comp, &sender_comp);
                                                                    fix.set_field(34, out_seq_num.to_string());
                                                                    out_seq_num += 1;
                                                                    let bytes = protocol::encode(fix);
                                                                    let _ = write_half.write_all(&bytes).await;
                                                                }
                                                            }
                                                        } else { break; }
                                                    }
                                                    res = read_half.read_buf(&mut read_buf) => {
                                                        match res {
                                                            Ok(0) => {
                                                                let senders = clients.read().await;
                                                                for tx in senders.iter() {
                                                                    let _ = tx.send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::PeerClosed }).await;
                                                                }
                                                                break;
                                                            }
                                                            Ok(_) => {
                                                                while let Some(msg_bytes) = protocol::try_extract_one(&mut read_buf) {
                                                                    last_rx = Instant::now();
                                                                    match protocol::decode(&msg_bytes) {
                                                                        Ok(msg) => {
                                                                            if let Some(seq) = msg.fields.get(&34) {
                                                                                if let Ok(seq_val) = seq.parse::<u32>() { _in_seq_num = seq_val; }
                                                                            }
                                                                            match msg.msg_type {
                                                                                FixMsgType::Logon => {
                                                                                    if let Some(hb) = msg.fields.get(&108) {
                                                                                        if let Ok(secs) = hb.parse::<u64>() { hb_interval = Duration::from_secs(secs); }
                                                                                    }
                                                                                    if let Some(s) = msg.fields.get(&49) { sender_comp = s.clone(); }
                                                                                    if let Some(t) = msg.fields.get(&56) { target_comp = t.clone(); }

                                                                                    // Validate using pluggable auth
                                                                                    if !auth.validate_logon(&sender_comp, &target_comp) {
                                                                                        let mut lo = protocol::build_logout(Some("Logon rejected"), &target_comp, &sender_comp);
                                                                                        lo.set_field(34, out_seq_num.to_string()); out_seq_num += 1;
                                                                                        let lo_bytes = protocol::encode(lo);
                                                                                        let _ = write_half.write_all(&lo_bytes).await;
                                                                                        let senders = clients.read().await;
                                                                                        for tx in senders.iter() {
                                                                                            let _ = tx.send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::ApplicationRequested }).await;
                                                                                        }
                                                                                        break;
                                                                                    }

                                                                                    // Echo logon
                                                                                    let mut logon = protocol::build_logon(hb_interval.as_secs() as u32, &target_comp, &sender_comp);
                                                                                    logon.set_field(34, out_seq_num.to_string()); out_seq_num += 1;
                                                                                    let bytes = protocol::encode(logon);
                                                                                    let _ = write_half.write_all(&bytes).await;

                                                                                    let senders = clients.read().await;
                                                                                    for tx in senders.iter() {
                                                                                        let _ = tx.send(GatewayEvent::SessionActive { session_id }).await;
                                                                                    }
                                                                                }
                                                                                FixMsgType::TestRequest => {
                                                                                    let id = msg.fields.get(&112).cloned();
                                                                                    let mut hb = protocol::build_heartbeat(id.as_deref(), &target_comp, &sender_comp);
                                                                                    hb.set_field(34, out_seq_num.to_string()); out_seq_num += 1;
                                                                                    let hb_bytes = protocol::encode(hb);
                                                                                    let _ = write_half.write_all(&hb_bytes).await;
                                                                                }
                                                                                FixMsgType::Logout => {
                                                                                    let mut lo = protocol::build_logout(None, &target_comp, &sender_comp);
                                                                                    lo.set_field(34, out_seq_num.to_string()); out_seq_num += 1;
                                                                                    let lo_bytes = protocol::encode(lo);
                                                                                    let _ = write_half.write_all(&lo_bytes).await;
                                                                                    let senders = clients.read().await;
                                                                                    for tx in senders.iter() {
                                                                                        let _ = tx.send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::ApplicationRequested }).await;
                                                                                    }
                                                                                    break;
                                                                                }
                                                                                FixMsgType::Heartbeat | FixMsgType::Unknown(_) => {}
                                                                                FixMsgType::ResendRequest | FixMsgType::SequenceReset => {}
                                                                            }
                                                                            let msg_type = match msg.msg_type { FixMsgType::Unknown(_) => "?".to_string(), _ => protocol::msg_type_as_str(&msg.msg_type).to_string() };
                                                                            let senders = clients.read().await;
                                                                            for tx in senders.iter() {
                                                                                let _ = tx.send(GatewayEvent::InboundMessage { session_id, msg_type: msg_type.clone(), payload: msg_bytes.clone() }).await;
                                                                            }
                                                                        }
                                                                        Err(_) => {
                                                                            let senders = clients.read().await;
                                                                            for tx in senders.iter() {
                                                                                let _ = tx.send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::ProtocolError }).await;
                                                                            }
                                                                            break;
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            Err(_) => {
                                                                let senders = clients.read().await;
                                                                for tx in senders.iter() {
                                                                    let _ = tx.send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::Unknown }).await;
                                                                }
                                                                break;
                                                            }
                                                        }
                                                    }
                                                    _ = tick.tick() => {
                                                        let idle = last_rx.elapsed();
                                                        if idle >= hb_interval * 3 {
                                                            let senders = clients.read().await;
                                                            for tx in senders.iter() {
                                                                let _ = tx.send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::Timeout }).await;
                                                            }
                                                            break;
                                                        } else if idle >= hb_interval * 2 {
                                                            if test_req_outstanding.is_none() {
                                                                let tr_id = format!("TR-{}", out_seq_num);
                                                                let mut tr = protocol::build_test_request(&tr_id, &target_comp, &sender_comp);
                                                                tr.set_field(34, out_seq_num.to_string()); out_seq_num += 1;
                                                                let tr_bytes = protocol::encode(tr);
                                                                let _ = write_half.write_all(&tr_bytes).await;
                                                                test_req_outstanding = Some(tr_id);
                                                            }
                                                        } else if idle >= hb_interval {
                                                            let mut hb = protocol::build_heartbeat(None, &target_comp, &sender_comp);
                                                            hb.set_field(34, out_seq_num.to_string()); out_seq_num += 1;
                                                            let hb_bytes = protocol::encode(hb);
                                                            let _ = write_half.write_all(&hb_bytes).await;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    });
                                }
                                Err(e) => {
                                    tracing::error!(error = %e, "Accept failed");
                                }
                            }
                        }
                    }
                });

                while let Some(cmd) = cmd_rx.recv().await {
                    match cmd {
                        GatewayCommand::RegisterClient {
                            library_id,
                            respond_to,
                        } => {
                            let (to_client_tx, to_client_rx) = mpsc::channel::<GatewayEvent>(1024);
                            let (from_client_tx, mut from_client_rx) =
                                mpsc::channel::<ClientCommand>(1024);

                            // Per-client task managing sessions and I/O
                            let to_client_tx_clone = to_client_tx.clone();
                            let next_id = Arc::clone(&next_session_id);
                            let store = store.clone();
                            {
                                let mut v = clients.write().await;
                                v.push(to_client_tx.clone());
                            }
                            let global_session_senders = Arc::clone(&global_session_senders);
                            tokio::spawn(async move {
                                let mut session_senders: HashMap<
                                    u64,
                                    mpsc::Sender<OutboundPayload>,
                                > = HashMap::new();

                                while let Some(cc) = from_client_rx.recv().await {
                                    match cc {
                                        ClientCommand::InitiateSession {
                                            host,
                                            port,
                                            sender_comp_id,
                                            target_comp_id,
                                            heartbeat_interval_secs,
                                            respond_to,
                                        } => {
                                            let addr = format!("{}:{}", host, port);
                                            match TcpStream::connect(addr).await {
                                                Ok(stream) => {
                                                    let session_id =
                                                        next_id.fetch_add(1, Ordering::Relaxed) + 1;
                                                    let (mut read_half, write_half) =
                                                        stream.into_split();

                                                    // Create channel for application-driven outbound payloads to this session task
                                                    let (app_out_tx, mut app_out_rx) =
                                                        mpsc::channel::<OutboundPayload>(1024);
                                                    session_senders
                                                        .insert(session_id, app_out_tx.clone());
                                                    {
                                                        let mut map =
                                                            global_session_senders.write().await;
                                                        map.insert(session_id, app_out_tx.clone());
                                                    }

                                                    // Spawn session task owning write half, performing handshake, timers, and parsing
                                                    let to_client_tx_reader =
                                                        to_client_tx_clone.clone();
                                                    let store = store.clone();
                                                    tokio::spawn(async move {
                                                        let mut write_half = write_half;
                                                        let hb_interval = Duration::from_secs(
                                                            heartbeat_interval_secs as u64,
                                                        );
                                                        let mut out_seq_num: u32 = 1;
                                                        let mut in_seq_num: u32 = 0;
                                                        let mut last_rx: Instant = Instant::now();
                                                        let mut test_req_outstanding: Option<
                                                            String,
                                                        > = None;
                                                        let sess_key = SessionKey {
                                                            sender_comp_id: sender_comp_id.clone(),
                                                            target_comp_id: target_comp_id.clone(),
                                                        };

                                                        // Send Logon
                                                        let mut logon = protocol::build_logon(
                                                            heartbeat_interval_secs,
                                                            &sender_comp_id,
                                                            &target_comp_id,
                                                        );
                                                        logon
                                                            .set_field(34, out_seq_num.to_string());
                                                        let seq_for_store = out_seq_num;
                                                        out_seq_num += 1;
                                                        let logon_bytes = protocol::encode(logon);
                                                        let _ = write_half
                                                            .write_all(&logon_bytes)
                                                            .await;
                                                        let _ = store
                                                            .append_bytes(
                                                                &sess_key,
                                                                Direction::Outbound,
                                                                Some(seq_for_store),
                                                                now_millis(),
                                                                logon_bytes.as_ref(),
                                                            )
                                                            .await;

                                                        // Timers
                                                        let mut interval =
                                                            time::interval(hb_interval);
                                                        interval.set_missed_tick_behavior(
                                                            time::MissedTickBehavior::Delay,
                                                        );

                                                        let mut read_buf =
                                                            BytesMut::with_capacity(16 * 1024);

                                                        loop {
                                                            tokio::select! {
                                                                biased;
                                                                // Application outbound payloads
                                                                maybe_out = app_out_rx.recv() => {
                                                                    if let Some(payload) = maybe_out {
                                                                        match payload {
                                                                            OutboundPayload::Raw(bytes) => {
                                                                                let _ = write_half.write_all(&bytes).await;
                                                                                let _ = store.append_bytes(&sess_key, Direction::Outbound, None, now_millis(), bytes.as_ref()).await;
                                                                            }
                                                                            OutboundPayload::Admin(msg) => {
                                                                                let mut fix = msg.into_fix(&sender_comp_id, &target_comp_id);
                                                                                fix.set_field(34, out_seq_num.to_string());
                                                                                let seq_for_store = out_seq_num;
                                                                                out_seq_num += 1;
                                                                                let bytes = protocol::encode(fix);
                                                                                let _ = write_half.write_all(&bytes).await;
                                                                                let _ = store.append_bytes(&sess_key, Direction::Outbound, Some(seq_for_store), now_millis(), bytes.as_ref()).await;
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
                                                                                    Ok(msg) => {
                                                                                        // Check seqnum if present
                                                                                        if let Some(seq) = msg.fields.get(&34) {
                                                                                            if let Ok(seq_val) = seq.parse::<u32>() { in_seq_num = seq_val; }
                                                                                        }
                                                                                        // Journal inbound
                                                                                        let inbound_seq = msg.fields.get(&34).and_then(|s| s.parse::<u32>().ok());
                                                                                        let _ = store.append_bytes(&sess_key, Direction::Inbound, inbound_seq, now_millis(), msg_bytes.as_ref()).await;

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
                                                                                                let seq_for_store = out_seq_num;
                                                                                                out_seq_num += 1;
                                                                                                let hb_bytes = protocol::encode(hb);
                                                                                                let _ = write_half.write_all(&hb_bytes).await;
                                                                                                let _ = store.append_bytes(&sess_key, Direction::Outbound, Some(seq_for_store), now_millis(), hb_bytes.as_ref()).await;
                                                                                            }
                                                                                            FixMsgType::ResendRequest => {
                                                                                                let begin = msg.fields.get(&7).and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                                                                                                let end = msg.fields.get(&16).and_then(|s| s.parse::<u32>().ok()).unwrap_or(in_seq_num);
                                                                                                if let Ok(chunks) = store.load_outbound_range(&sess_key, begin, end).await {
                                                                                                    for b in chunks {
                                                                                                        if let Ok(mut m) = protocol::decode(&b) {
                                                                                                            // Mark as possible duplicate and set OrigSendingTime
                                                                                                            m.set_field(43, "Y");
                                                                                                            m.set_field(122, format!("{}", now_millis()));
                                                                                                            let new_b = protocol::encode(m);
                                                                                                            let _ = write_half.write_all(&new_b).await;
                                                                                                        } else {
                                                                                                            let _ = write_half.write_all(&b).await;
                                                                                                        }
                                                                                                    }
                                                                                                }
                                                                                            }
                                                                                            FixMsgType::Logout => {
                                                                                                // Echo logout and close
                                                                                                let mut lo = protocol::build_logout(None, &sender_comp_id, &target_comp_id);
                                                                                                lo.set_field(34, out_seq_num.to_string());
                                                                                                let seq_for_store = out_seq_num;
                                                                                                out_seq_num += 1;
                                                                                                let lo_bytes = protocol::encode(lo);
                                                                                                let _ = write_half.write_all(&lo_bytes).await;
                                                                                                let _ = store.append_bytes(&sess_key, Direction::Outbound, Some(seq_for_store), now_millis(), lo_bytes.as_ref()).await;
                                                                                                let _ = to_client_tx_reader
                                                                                                    .send(GatewayEvent::Disconnected { session_id, reason: DisconnectReason::ApplicationRequested })
                                                                                                    .await;
                                                                                                break;
                                                                                            }
                                                                                            FixMsgType::SequenceReset | FixMsgType::Unknown(_) => {}
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
                                                                            let seq_for_store = out_seq_num;
                                                                            out_seq_num += 1;
                                                                            let tr_bytes = protocol::encode(tr);
                                                                            let _ = write_half.write_all(&tr_bytes).await;
                                                                            let _ = store.append_bytes(&sess_key, Direction::Outbound, Some(seq_for_store), now_millis(), tr_bytes.as_ref()).await;
                                                                            test_req_outstanding = Some(tr_id);
                                                                        }
                                                                    } else if idle >= hb_interval {
                                                                        let mut hb = protocol::build_heartbeat(None, &sender_comp_id, &target_comp_id);
                                                                        hb.set_field(34, out_seq_num.to_string());
                                                                        let seq_for_store = out_seq_num;
                                                                        out_seq_num += 1;
                                                                        let hb_bytes = protocol::encode(hb);
                                                                        let _ = write_half.write_all(&hb_bytes).await;
                                                                        let _ = store.append_bytes(&sess_key, Direction::Outbound, Some(seq_for_store), now_millis(), hb_bytes.as_ref()).await;
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    });

                                                    let _ = to_client_tx_clone
                                                        .send(GatewayEvent::SessionActive {
                                                            session_id,
                                                        })
                                                        .await;
                                                    let _ = respond_to
                                                        .send(SessionHandle { session_id });
                                                }
                                                Err(e) => {
                                                    let _ = respond_to
                                                        .send(SessionHandle { session_id: 0 });
                                                    let _ = to_client_tx_clone
                                                        .send(GatewayEvent::Disconnected {
                                                            session_id: 0,
                                                            reason: DisconnectReason::Unknown,
                                                        })
                                                        .await;
                                                    tracing::error!(error = %e, "Failed to connect session");
                                                }
                                            }
                                        }
                                        ClientCommand::Send {
                                            session_id,
                                            payload,
                                        } => {
                                            if let Some(tx) = session_senders.get_mut(&session_id) {
                                                let _ =
                                                    tx.send(OutboundPayload::Raw(payload)).await;
                                            }
                                        }
                                        ClientCommand::SendAdmin {
                                            session_id, msg, ..
                                        } => {
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

                            _clients.push(ClientConnectionInternal {
                                _library_id: library_id,
                                _to_client_tx: to_client_tx,
                            });
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
    _to_client_tx: mpsc::Sender<GatewayEvent>,
}

#[derive(Debug)]
pub enum GatewayEvent {
    SessionActive {
        session_id: u64,
    },
    InboundMessage {
        session_id: u64,
        msg_type: String,
        payload: Bytes,
    },
    Disconnected {
        session_id: u64,
        reason: DisconnectReason,
    },
}

#[derive(Debug)]
pub enum GatewayCommand {
    RegisterClient {
        library_id: i32,
        respond_to: oneshot::Sender<ClientConnection>,
    },
    Shutdown,
}

#[derive(Debug)]
pub enum ClientCommand {
    InitiateSession {
        host: String,
        port: u16,
        sender_comp_id: String,
        target_comp_id: String,
        heartbeat_interval_secs: u32,
        respond_to: oneshot::Sender<SessionHandle>,
    },
    Send {
        session_id: u64,
        payload: Bytes,
    },
    SendAdmin {
        session_id: u64,
        msg: AdminMessage,
        sender_comp_id: String,
        target_comp_id: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct SessionHandle {
    pub session_id: u64,
}

// Re-export for client module
pub(crate) use ClientCommand as GatewayClientCommand;
pub(crate) use GatewayEvent as GatewayToClientEvent;
pub(crate) use SessionHandle as GatewaySessionHandle;
