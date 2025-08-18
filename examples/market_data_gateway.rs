
//! Market Data Gateway Example
//! 
//! This example demonstrates a high-frequency market data gateway that:
//! - Accepts multiple client connections
//! - Distributes market data to subscribed clients
//! - Handles session lifecycle management
//! - Demonstrates backpressure handling
//! 
//! Based on Artio's market data gateway patterns.

use async_trait::async_trait;
use fixg::messages::AdminMessage;
use fixg::session::SessionConfig;
use fixg::{
    FixClient, FixClientConfig, FixHandler, Gateway, GatewayConfig, InboundMessage, Session,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio::time::{self, Duration, Instant};

/// Market data distributor that manages client subscriptions
#[derive(Clone)]
struct MarketDataDistributor {
    /// Broadcast channel for market data
    market_data_tx: broadcast::Sender<MarketData>,
    /// Active client sessions
    clients: Arc<Mutex<HashMap<String, Arc<Session>>>>,
}

#[derive(Debug, Clone)]
struct MarketData {
    symbol: String,
    price: f64,
    quantity: u64,
    timestamp: Instant,
}

impl MarketDataDistributor {
    fn new() -> Self {
        let (market_data_tx, _) = broadcast::channel(10000);
        Self {
            market_data_tx,
            clients: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn add_client(&self, session_id: String, session: Arc<Session>) {
        let mut clients = self.clients.lock().await;
        clients.insert(session_id.clone(), session);
        println!("Added client: {}", session_id);
    }

    async fn remove_client(&self, session_id: &str) {
        let mut clients = self.clients.lock().await;
        clients.remove(session_id);
        println!("Removed client: {}", session_id);
    }

    async fn publish_market_data(&self, data: MarketData) {
        // Send via broadcast channel (for demonstration)
        if let Err(_) = self.market_data_tx.send(data.clone()) {
            println!("No active subscribers for market data");
            return;
        }

        // Also send directly to FIX clients
        let clients = self.clients.lock().await;
        for (client_id, session) in clients.iter() {
            let fix_message = format!(
                "35=D|55={}|44={}|38={}|52={}",
                data.symbol,
                data.price,
                data.quantity,
                data.timestamp.elapsed().as_millis()
            );

            // Send as raw FIX message (in real implementation, use generated types)
            if let Err(e) = session.send_raw(fix_message.as_bytes()).await {
                println!("Failed to send to client {}: {}", client_id, e);
            }
        }
    }

    async fn start_market_data_feed(&self) {
        let distributor = self.clone();
        tokio::spawn(async move {
            let symbols = vec!["EURUSD", "GBPUSD", "USDJPY", "AUDUSD"];
            let mut interval = time::interval(Duration::from_millis(10)); // 100Hz
            let mut counter = 0u64;

            loop {
                interval.tick().await;
                
                for symbol in &symbols {
                    let data = MarketData {
                        symbol: symbol.to_string(),
                        price: 1.0 + (counter as f64 * 0.0001) % 0.1,
                        quantity: 1000000 + (counter % 1000),
                        timestamp: Instant::now(),
                    };
                    
                    distributor.publish_market_data(data).await;
                    counter += 1;
                }
            }
        });
    }
}

/// FIX handler for market data clients
struct MarketDataHandler {
    distributor: MarketDataDistributor,
    session_id: String,
    market_data_rx: Option<broadcast::Receiver<MarketData>>,
}

impl MarketDataHandler {
    fn new(distributor: MarketDataDistributor, session_id: String) -> Self {
        let market_data_rx = Some(distributor.market_data_tx.subscribe());
        Self {
            distributor,
            session_id,
            market_data_rx,
        }
    }
}

#[async_trait]
impl FixHandler for MarketDataHandler {
    async fn on_session_active(&mut self, session: &Session) {
        println!("Market data session active: {}", self.session_id);
        
        // Register this client with the distributor
        self.distributor
            .add_client(self.session_id.clone(), Arc::new(session.clone()))
            .await;

        // Send initial heartbeat
        let _ = session
            .send_admin(AdminMessage::TestRequest {
                id: "INITIAL".to_string(),
            })
            .await;

        // Start listening for market data broadcasts
        if let Some(mut rx) = self.market_data_rx.take() {
            let session = session.clone();
            let session_id = self.session_id.clone();
            
            tokio::spawn(async move {
                while let Ok(data) = rx.recv().await {
                    // Process market data and potentially send to client
                    // This is just for demonstration - real implementation would
                    // handle subscriptions, filtering, etc.
                }
            });
        }
    }

    async fn on_message(&mut self, session: &Session, msg: InboundMessage) {
        if let Some(admin) = msg.admin() {
            match admin {
                AdminMessage::TestRequest { id } => {
                    println!("Received test request from {}: {}", self.session_id, id);
                    // Respond with heartbeat
                    let _ = session
                        .send_admin(AdminMessage::Heartbeat {
                            test_req_id: Some(id.clone()),
                        })
                        .await;
                }
                AdminMessage::Logout { text } => {
                    println!("Client {} logging out: {:?}", self.session_id, text);
                }
                _ => {
                    println!("Received admin message from {}: {:?}", self.session_id, admin);
                }
            }
        } else {
            // Handle application messages (market data subscriptions, etc.)
            println!(
                "Received application message from {}: {} bytes",
                self.session_id,
                msg.body().len()
            );
            
            // Parse and handle subscription requests
            // In a real implementation, this would parse FIX messages
            // and manage client subscriptions
        }
    }

    async fn on_session_disconnect(&mut self, _session: &Session, reason: fixg::DisconnectReason) {
        println!("Client {} disconnected: {:?}", self.session_id, reason);
        self.distributor.remove_client(&self.session_id).await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting Market Data Gateway (Artio-style)");

    // Create market data distributor
    let distributor = MarketDataDistributor::new();

    // Start market data feed
    distributor.start_market_data_feed().await;

    // Configure gateway to accept connections
    let gateway_config = GatewayConfig {
        bind_address: "0.0.0.0:9876".parse()?,
        ..Default::default()
    };

    let gateway_handle = Gateway::spawn(gateway_config).await?;
    println!("Gateway listening on 0.0.0.0:9876");

    // Simulate multiple client connections
    let num_clients = 3;
    let mut client_tasks = Vec::new();

    for i in 0..num_clients {
        let gateway_handle = gateway_handle.clone();
        let distributor = distributor.clone();
        
        let task = tokio::spawn(async move {
            let session_id = format!("MDClient{}", i + 1);
            
            // Configure client
            let client_config = FixClientConfig::new(100 + i as i32);
            let mut client = FixClient::connect(client_config, gateway_handle).await?;

            // Configure session
            let session_config = SessionConfig::builder()
                .host("127.0.0.1")
                .port(9876)
                .sender_comp_id(&session_id)
                .target_comp_id("MDGATEWAY")
                .heartbeat_interval_secs(30)
                .build()?;

            let _session = client.initiate(session_config).await?;

            // Create handler and run client
            let handler = MarketDataHandler::new(distributor, session_id.clone());
            println!("Starting client: {}", session_id);
            
            client.run(handler).await
        });

        client_tasks.push(task);
    }

    // Wait for all clients
    println!("Market data gateway running with {} clients", num_clients);
    println!("Press Ctrl+C to stop");

    // Wait for any client to finish (or error)
    let (result, _remaining) = futures::future::select_all(client_tasks).await;
    
    match result {
        Ok(Ok(())) => println!("Client completed successfully"),
        Ok(Err(e)) => println!("Client error: {}", e),
        Err(e) => println!("Task join error: {}", e),
    }

    Ok(())
}
