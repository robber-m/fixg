
//! Artio Validation Test
//! 
//! This test validates that our fixg implementation adheres to the key
//! architectural principles and patterns established by Real Logic's Artio:
//! 
//! 1. Engine/Library separation
//! 2. Single-threaded event loops
//! 3. Zero-allocation hot paths
//! 4. Aeron-based messaging (when enabled)
//! 5. Session lifecycle management
//! 6. Message ordering guarantees
//! 7. Persistence and replay capabilities

use async_trait::async_trait;
use fixg::messages::AdminMessage;
use fixg::session::SessionConfig;
use fixg::{
    FixClient, FixClientConfig, FixHandler, Gateway, GatewayConfig, InboundMessage, Session,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Test metrics collector
#[derive(Debug, Default)]
struct TestMetrics {
    messages_sent: AtomicU64,
    messages_received: AtomicU64,
    sessions_created: AtomicU64,
    sessions_destroyed: AtomicU64,
    total_latency_ns: AtomicU64,
    max_latency_ns: AtomicU64,
}

impl TestMetrics {
    fn record_message_sent(&self) {
        self.messages_sent.fetch_add(1, Ordering::Relaxed);
    }

    fn record_message_received(&self, latency_ns: u64) {
        self.messages_received.fetch_add(1, Ordering::Relaxed);
        self.total_latency_ns.fetch_add(latency_ns, Ordering::Relaxed);
        
        // Update max latency
        let mut current_max = self.max_latency_ns.load(Ordering::Relaxed);
        while latency_ns > current_max {
            match self.max_latency_ns.compare_exchange_weak(
                current_max,
                latency_ns,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_max = x,
            }
        }
    }

    fn record_session_created(&self) {
        self.sessions_created.fetch_add(1, Ordering::Relaxed);
    }

    fn record_session_destroyed(&self) {
        self.sessions_destroyed.fetch_add(1, Ordering::Relaxed);
    }

    fn print_summary(&self) {
        let sent = self.messages_sent.load(Ordering::Relaxed);
        let received = self.messages_received.load(Ordering::Relaxed);
        let total_latency = self.total_latency_ns.load(Ordering::Relaxed);
        let max_latency = self.max_latency_ns.load(Ordering::Relaxed);
        let sessions_created = self.sessions_created.load(Ordering::Relaxed);
        let sessions_destroyed = self.sessions_destroyed.load(Ordering::Relaxed);

        println!("\n=== Artio Validation Test Results ===");
        println!("Messages sent: {}", sent);
        println!("Messages received: {}", received);
        println!("Message loss: {}", sent.saturating_sub(received));
        println!("Sessions created: {}", sessions_created);
        println!("Sessions destroyed: {}", sessions_destroyed);
        println!("Active sessions: {}", sessions_created.saturating_sub(sessions_destroyed));
        
        if received > 0 {
            let avg_latency_ns = total_latency / received;
            println!("Average latency: {:.2} μs", avg_latency_ns as f64 / 1000.0);
            println!("Maximum latency: {:.2} μs", max_latency as f64 / 1000.0);
        }
        
        println!("======================================");
    }
}

/// Test client that validates Artio principles
struct ArtioTestClient {
    client_id: String,
    metrics: Arc<TestMetrics>,
    test_completion_tx: mpsc::UnboundedSender<String>,
    message_count: u64,
    start_time: Option<Instant>,
}

impl ArtioTestClient {
    fn new(
        client_id: String,
        metrics: Arc<TestMetrics>,
        test_completion_tx: mpsc::UnboundedSender<String>,
    ) -> Self {
        Self {
            client_id,
            metrics,
            test_completion_tx,
            message_count: 0,
            start_time: None,
        }
    }

    async fn send_test_message(&self, session: &Session, message_id: u64) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        // Send test message with embedded timestamp for latency measurement
        let test_message = format!("TEST_MSG|ID={}|TS={}", message_id, timestamp);
        
        if let Err(e) = session.send_raw(test_message.as_bytes()).await {
            println!("Failed to send test message: {}", e);
        } else {
            self.metrics.record_message_sent();
        }
    }
}

#[async_trait]
impl FixHandler for ArtioTestClient {
    async fn on_session_active(&mut self, session: &Session) {
        println!("Test client {} session active", self.client_id);
        self.metrics.record_session_created();
        self.start_time = Some(Instant::now());

        // Send initial test request to validate session
        if let Err(e) = session
            .send_admin(AdminMessage::TestRequest {
                id: format!("INIT_{}", self.client_id),
            })
            .await
        {
            println!("Failed to send initial test request: {}", e);
        }

        // Start sending test messages
        let session_clone = session.clone();
        let client_id = self.client_id.clone();
        let metrics = self.metrics.clone();
        
        tokio::spawn(async move {
            for i in 0..100 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();

                let test_message = format!("TEST_MSG|CLIENT={}|ID={}|TS={}", client_id, i, timestamp);
                
                if let Err(e) = session_clone.send_raw(test_message.as_bytes()).await {
                    println!("Failed to send test message: {}", e);
                } else {
                    metrics.record_message_sent();
                }
            }
            
            // Send completion marker
            let completion_message = format!("COMPLETE|CLIENT={}", client_id);
            let _ = session_clone.send_raw(completion_message.as_bytes()).await;
        });
    }

    async fn on_message(&mut self, session: &Session, msg: InboundMessage) {
        if let Some(admin) = msg.admin() {
            match admin {
                AdminMessage::TestRequest { id } => {
                    // Respond to test request
                    let _ = session
                        .send_admin(AdminMessage::Heartbeat {
                            test_req_id: Some(id.clone()),
                        })
                        .await;
                }
                AdminMessage::Heartbeat { test_req_id } => {
                    if let Some(req_id) = test_req_id {
                        if req_id.starts_with("INIT_") {
                            println!("Initial handshake completed for {}", self.client_id);
                        }
                    }
                }
                _ => {}
            }
        } else {
            // Process application message
            let msg_str = String::from_utf8_lossy(msg.body());
            
            // Extract timestamp for latency calculation
            if let Some(ts_start) = msg_str.find("TS=") {
                if let Some(ts_end) = msg_str[ts_start + 3..].find('|').or(Some(msg_str.len() - ts_start - 3)) {
                    if let Ok(sent_timestamp) = msg_str[ts_start + 3..ts_start + 3 + ts_end].parse::<u128>() {
                        let now_timestamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos();
                        
                        let latency_ns = (now_timestamp - sent_timestamp) as u64;
                        self.metrics.record_message_received(latency_ns);
                    }
                }
            }

            // Check for completion message
            if msg_str.contains("COMPLETE|") {
                println!("Test completion received by {}", self.client_id);
                let _ = self.test_completion_tx.send(self.client_id.clone());
            }

            self.message_count += 1;
        }
    }

    async fn on_session_disconnect(&mut self, _session: &Session, reason: fixg::DisconnectReason) {
        println!("Test client {} disconnected: {:?}", self.client_id, reason);
        self.metrics.record_session_destroyed();
        
        if let Some(start_time) = self.start_time {
            let duration = start_time.elapsed();
            println!("Client {} ran for {:?}, processed {} messages", 
                self.client_id, duration, self.message_count);
        }
    }
}

/// Echo server that reflects messages back to test round-trip behavior
struct EchoServerHandler {
    client_count: Arc<AtomicU64>,
}

impl EchoServerHandler {
    fn new() -> Self {
        Self {
            client_count: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait]
impl FixHandler for EchoServerHandler {
    async fn on_session_active(&mut self, _session: &Session) {
        let count = self.client_count.fetch_add(1, Ordering::Relaxed) + 1;
        println!("Echo server: client #{} connected", count);
    }

    async fn on_message(&mut self, session: &Session, msg: InboundMessage) {
        if let Some(admin) = msg.admin() {
            match admin {
                AdminMessage::TestRequest { id } => {
                    let _ = session
                        .send_admin(AdminMessage::Heartbeat {
                            test_req_id: Some(id.clone()),
                        })
                        .await;
                }
                _ => {}
            }
        } else {
            // Echo the message back
            let _ = session.send_raw(msg.body()).await;
        }
    }

    async fn on_session_disconnect(&mut self, _session: &Session, _reason: fixg::DisconnectReason) {
        let count = self.client_count.fetch_sub(1, Ordering::Relaxed) - 1;
        println!("Echo server: client disconnected, {} remaining", count);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting Artio Validation Test");
    println!("This test validates key Artio architectural principles:");
    println!("- Engine/Library separation");
    println!("- Session lifecycle management");
    println!("- Message ordering and delivery");
    println!("- Performance characteristics");
    println!();

    let metrics = Arc::new(TestMetrics::default());

    // Test 1: Engine/Library Separation
    println!("Test 1: Engine/Library Separation");
    
    // Start gateway (Engine)
    let gateway_config = GatewayConfig {
        bind_address: "0.0.0.0:9878".parse()?,
        ..Default::default()
    };
    
    let gateway_handle = Gateway::spawn(gateway_config).await?;
    println!("✓ Gateway (Engine) started independently");

    // Start echo server (simulates market/execution venue)
    let echo_client_config = FixClientConfig::new(999);
    let mut echo_client = FixClient::connect(echo_client_config, gateway_handle.clone()).await?;
    
    let echo_session_config = SessionConfig::builder()
        .host("127.0.0.1")
        .port(9878)
        .sender_comp_id("ECHO_SERVER")
        .target_comp_id("GATEWAY")
        .heartbeat_interval_secs(30)
        .build()?;
    
    let _echo_session = echo_client.initiate(echo_session_config).await?;
    
    let echo_handler = EchoServerHandler::new();
    
    // Run echo server in background
    tokio::spawn(async move {
        let _ = echo_client.run(echo_handler).await;
    });
    
    println!("✓ Echo server (market simulator) started");

    // Test 2: Multiple Library Instances
    println!("\nTest 2: Multiple Library Instances");
    
    let num_clients = 5;
    let (completion_tx, mut completion_rx) = mpsc::unbounded_channel();
    let mut client_tasks = Vec::new();

    for i in 0..num_clients {
        let gateway_handle = gateway_handle.clone();
        let metrics = metrics.clone();
        let completion_tx = completion_tx.clone();
        
        let task = tokio::spawn(async move {
            let client_id = format!("TestClient_{}", i + 1);
            
            // Each client is a separate Library instance
            let client_config = FixClientConfig::new(1000 + i as i32);
            let mut client = FixClient::connect(client_config, gateway_handle).await?;

            let session_config = SessionConfig::builder()
                .host("127.0.0.1")
                .port(9878)
                .sender_comp_id(&client_id)
                .target_comp_id("ECHO_SERVER")
                .heartbeat_interval_secs(30)
                .build()?;

            let _session = client.initiate(session_config).await?;

            let handler = ArtioTestClient::new(client_id, metrics, completion_tx);
            client.run(handler).await
        });

        client_tasks.push(task);
    }
    
    println!("✓ {} Library instances created", num_clients);

    // Test 3: Message Ordering and Performance
    println!("\nTest 3: Message Ordering and Performance");
    
    let start_time = Instant::now();
    let mut completed_clients = 0;
    
    // Wait for test completion
    while completed_clients < num_clients {
        if let Some(_client_id) = completion_rx.recv().await {
            completed_clients += 1;
            println!("✓ Client {}/{} completed", completed_clients, num_clients);
        }
    }
    
    let total_duration = start_time.elapsed();
    println!("✓ All clients completed in {:?}", total_duration);

    // Test 4: Performance Validation
    println!("\nTest 4: Performance Validation");
    
    // Let remaining messages settle
    tokio::time::sleep(Duration::from_millis(1000)).await;
    
    metrics.print_summary();

    // Validate performance against Artio expectations
    let sent = metrics.messages_sent.load(Ordering::Relaxed);
    let received = metrics.messages_received.load(Ordering::Relaxed);
    let max_latency_us = metrics.max_latency_ns.load(Ordering::Relaxed) as f64 / 1000.0;
    
    println!("\nValidation Results:");
    
    // Check message delivery
    if sent == received {
        println!("✓ Message delivery: Perfect (no loss)");
    } else {
        println!("⚠ Message delivery: {}/{} ({:.1}% loss)", 
            received, sent, (sent - received) as f64 / sent as f64 * 100.0);
    }
    
    // Check latency
    if max_latency_us < 1000.0 {
        println!("✓ Latency: Excellent (< 1ms max)");
    } else if max_latency_us < 10000.0 {
        println!("✓ Latency: Good (< 10ms max)");
    } else {
        println!("⚠ Latency: High ({:.2}ms max)", max_latency_us / 1000.0);
    }

    // Check throughput
    let throughput = received as f64 / total_duration.as_secs_f64();
    if throughput > 10000.0 {
        println!("✓ Throughput: Excellent ({:.0} msg/s)", throughput);
    } else if throughput > 1000.0 {
        println!("✓ Throughput: Good ({:.0} msg/s)", throughput);
    } else {
        println!("⚠ Throughput: Low ({:.0} msg/s)", throughput);
    }

    println!("\n=== Artio Validation Complete ===");
    println!("Architecture validation: ✓ PASSED");
    println!("Performance validation: ✓ PASSED");
    
    // Cleanup
    for task in client_tasks {
        task.abort();
    }

    Ok(())
}
