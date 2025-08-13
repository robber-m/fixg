use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{self, Duration, Instant};

use fixg::protocol::{self, FixMsgType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(("127.0.0.1", 9876)).await?;
    println!("Toy acceptor listening on 127.0.0.1:9876");

    loop {
        let (mut socket, addr) = listener.accept().await?;
        println!("Accepted connection from {}", addr);

        tokio::spawn(async move {
            let mut read_buf = BytesMut::with_capacity(16 * 1024);
            let mut out_seq_num: u32 = 1;
            let mut last_rx = Instant::now();
            let mut hb_interval = Duration::from_secs(30);
            let mut sender_comp = String::from("ACCEPTOR");
            let mut target_comp = String::from("INITIATOR");

            // After logon exchange, run heartbeat loop
            let mut interval = time::interval(Duration::from_secs(1));

            loop {
                tokio::select! {
                    res = socket.read_buf(&mut read_buf) => {
                        match res {
                            Ok(0) => { println!("peer closed"); break; }
                            Ok(_) => {
                                while let Some(msg_bytes) = protocol::try_extract_one(&mut read_buf) {
                                    last_rx = Instant::now();
                                    match protocol::decode(&msg_bytes) {
                                        Ok(mut msg) => {
                                            match msg.msg_type {
                                                FixMsgType::Logon => {
                                                    // Capture hb interval and comp ids
                                                    if let Some(hb) = msg.fields.get(&108) {
                                                        if let Ok(secs) = hb.parse::<u64>() { hb_interval = Duration::from_secs(secs); }
                                                    }
                                                    if let Some(s) = msg.fields.get(&49) { target_comp = s.clone(); }
                                                    if let Some(t) = msg.fields.get(&56) { sender_comp = t.clone(); }
                                                    // Respond with logon
                                                    let mut logon = protocol::build_logon(hb_interval.as_secs() as u32, &sender_comp, &target_comp);
                                                    logon.set_field(34, out_seq_num.to_string()); out_seq_num += 1;
                                                    let bytes = protocol::encode(logon);
                                                    let _ = socket.write_all(&bytes).await;
                                                }
                                                FixMsgType::TestRequest => {
                                                    let id = msg.fields.get(&112).cloned();
                                                    let mut hb = protocol::build_heartbeat(id.as_deref(), &sender_comp, &target_comp);
                                                    hb.set_field(34, out_seq_num.to_string()); out_seq_num += 1;
                                                    let bytes = protocol::encode(hb);
                                                    let _ = socket.write_all(&bytes).await;
                                                }
                                                FixMsgType::Logout => {
                                                    let mut lo = protocol::build_logout(None, &sender_comp, &target_comp);
                                                    lo.set_field(34, out_seq_num.to_string()); out_seq_num += 1;
                                                    let bytes = protocol::encode(lo);
                                                    let _ = socket.write_all(&bytes).await;
                                                    let _ = socket.shutdown().await;
                                                    return;
                                                }
                                                FixMsgType::Heartbeat | FixMsgType::Unknown(_) => {}
                                            }
                                        }
                                        Err(e) => { println!("protocol error: {}", e); return; }
                                    }
                                }
                            }
                            Err(_) => { println!("read error"); break; }
                        }
                    }
                    _ = interval.tick() => {
                        let idle = last_rx.elapsed();
                        if idle >= hb_interval {
                            let mut hb = protocol::build_heartbeat(None, &sender_comp, &target_comp);
                            hb.set_field(34, out_seq_num.to_string()); out_seq_num += 1;
                            let bytes = protocol::encode(hb);
                            let _ = socket.write_all(&bytes).await;
                            last_rx = Instant::now();
                        }
                    }
                }
            }
        });
    }
}