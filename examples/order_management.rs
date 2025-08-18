
//! Order Management System Example
//! 
//! Demonstrates a complete order lifecycle management system similar to
//! Artio's trading examples, featuring:
//! - Order entry and validation
//! - Execution reports
//! - Order book management
//! - Risk checks
//! - State persistence

use async_trait::async_trait;
use fixg::messages::{AdminMessage, ExecutionReport, ExecType, OrdStatus};
use fixg::session::SessionConfig;
use fixg::{
    FixClient, FixClientConfig, FixHandler, Gateway, GatewayConfig, InboundMessage, Session,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct Order {
    cl_ord_id: String,
    order_id: String,
    symbol: String,
    side: char, // '1' = Buy, '2' = Sell
    order_qty: u64,
    price: Option<f64>,
    ord_type: char, // '1' = Market, '2' = Limit
    ord_status: OrdStatus,
    cum_qty: u64,
    leaves_qty: u64,
    avg_px: f64,
}

impl Order {
    fn new(cl_ord_id: String, symbol: String, side: char, order_qty: u64, price: Option<f64>, ord_type: char) -> Self {
        Self {
            cl_ord_id,
            order_id: Uuid::new_v4().to_string(),
            symbol,
            side,
            order_qty,
            price,
            ord_type,
            ord_status: OrdStatus::New,
            cum_qty: 0,
            leaves_qty: order_qty,
            avg_px: 0.0,
        }
    }

    fn fill(&mut self, fill_qty: u64, fill_price: f64) {
        let old_cum_qty = self.cum_qty;
        self.cum_qty += fill_qty;
        self.leaves_qty = self.order_qty.saturating_sub(self.cum_qty);

        // Update average price
        if self.cum_qty > 0 {
            self.avg_px = ((self.avg_px * old_cum_qty as f64) + (fill_price * fill_qty as f64)) / self.cum_qty as f64;
        }

        // Update status
        if self.leaves_qty == 0 {
            self.ord_status = OrdStatus::Filled;
        } else {
            self.ord_status = OrdStatus::PartiallyFilled;
        }
    }
}

/// Order Management System that handles order lifecycle
#[derive(Clone)]
struct OrderManager {
    orders: Arc<Mutex<HashMap<String, Order>>>,
    session: Option<Arc<Session>>,
}

impl OrderManager {
    fn new() -> Self {
        Self {
            orders: Arc::new(Mutex::new(HashMap::new())),
            session: None,
        }
    }

    async fn set_session(&mut self, session: Arc<Session>) {
        self.session = Some(session);
    }

    async fn handle_new_order(&self, cl_ord_id: String, symbol: String, side: char, qty: u64, price: Option<f64>, ord_type: char) -> Result<(), String> {
        // Risk checks
        if qty == 0 {
            return Err("Invalid quantity".to_string());
        }

        if ord_type == '2' && price.is_none() {
            return Err("Limit orders require a price".to_string());
        }

        // Create order
        let mut order = Order::new(cl_ord_id.clone(), symbol, side, qty, price, ord_type);
        order.ord_status = OrdStatus::New;

        // Store order
        {
            let mut orders = self.orders.lock().await;
            orders.insert(cl_ord_id.clone(), order.clone());
        }

        // Send acknowledgment
        self.send_execution_report(&order, ExecType::New).await?;

        // For demonstration, immediately start working the order
        self.start_order_execution(cl_ord_id).await;

        Ok(())
    }

    async fn start_order_execution(&self, cl_ord_id: String) {
        let order_manager = self.clone();
        
        tokio::spawn(async move {
            // Simulate order execution delay
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            let mut orders = order_manager.orders.lock().await;
            if let Some(order) = orders.get_mut(&cl_ord_id) {
                if order.ord_status == OrdStatus::New {
                    // Simulate partial fill
                    let fill_qty = std::cmp::min(order.leaves_qty, order.order_qty / 3);
                    let fill_price = order.price.unwrap_or(100.0); // Use order price or market price

                    order.fill(fill_qty, fill_price);
                    let updated_order = order.clone();
                    drop(orders); // Release lock before async call

                    // Send execution report
                    let _ = order_manager.send_execution_report(&updated_order, ExecType::PartialFill).await;

                    // If there are leaves, schedule another fill
                    if updated_order.leaves_qty > 0 {
                        let order_manager_clone = order_manager.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                            order_manager_clone.complete_order_execution(cl_ord_id).await;
                        });
                    }
                }
            }
        });
    }

    async fn complete_order_execution(&self, cl_ord_id: String) {
        let mut orders = self.orders.lock().await;
        if let Some(order) = orders.get_mut(&cl_ord_id) {
            if order.leaves_qty > 0 {
                let fill_price = order.price.unwrap_or(100.0);
                order.fill(order.leaves_qty, fill_price);
                let updated_order = order.clone();
                drop(orders);

                let _ = self.send_execution_report(&updated_order, ExecType::Fill).await;
            }
        }
    }

    async fn send_execution_report(&self, order: &Order, exec_type: ExecType) -> Result<(), String> {
        if let Some(session) = &self.session {
            let exec_report = ExecutionReport {
                order_id: order.order_id.clone(),
                cl_ord_id: order.cl_ord_id.clone(),
                exec_id: Uuid::new_v4().to_string(),
                exec_type,
                ord_status: order.ord_status.clone(),
                symbol: order.symbol.clone(),
                side: order.side,
                leaves_qty: order.leaves_qty,
                cum_qty: order.cum_qty,
                avg_px: order.avg_px,
            };

            // In a real implementation, this would use generated message encoding
            let fix_message = format!(
                "35=8|11={}|37={}|17={}|150={}|39={}|55={}|54={}|151={}|14={}|6={}",
                exec_report.cl_ord_id,
                exec_report.order_id,
                exec_report.exec_id,
                exec_type as u8,
                order.ord_status as u8,
                exec_report.symbol,
                exec_report.side,
                exec_report.leaves_qty,
                exec_report.cum_qty,
                exec_report.avg_px
            );

            session.send_raw(fix_message.as_bytes()).await
                .map_err(|e| format!("Failed to send execution report: {}", e))?;

            println!("Sent execution report: {:?} for order {}", exec_type, order.cl_ord_id);
        }

        Ok(())
    }

    async fn handle_order_cancel(&self, cl_ord_id: String) -> Result<(), String> {
        let mut orders = self.orders.lock().await;
        if let Some(order) = orders.get_mut(&cl_ord_id) {
            if order.ord_status == OrdStatus::New || order.ord_status == OrdStatus::PartiallyFilled {
                order.ord_status = OrdStatus::Canceled;
                let updated_order = order.clone();
                drop(orders);

                self.send_execution_report(&updated_order, ExecType::Canceled).await?;
                return Ok(());
            }
        }

        Err("Order not found or cannot be canceled".to_string())
    }
}

/// Trading client handler
struct TradingHandler {
    order_manager: OrderManager,
    client_id: String,
}

impl TradingHandler {
    fn new(client_id: String) -> Self {
        Self {
            order_manager: OrderManager::new(),
            client_id,
        }
    }
}

#[async_trait]
impl FixHandler for TradingHandler {
    async fn on_session_active(&mut self, session: &Session) {
        println!("Trading session active for client: {}", self.client_id);
        self.order_manager.set_session(Arc::new(session.clone())).await;

        // Send initial heartbeat
        let _ = session
            .send_admin(AdminMessage::TestRequest {
                id: "TRADING_READY".to_string(),
            })
            .await;
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
                _ => {
                    println!("Received admin message: {:?}", admin);
                }
            }
        } else {
            // Parse application messages (in real implementation, use generated types)
            let msg_str = String::from_utf8_lossy(msg.body());
            println!("Received FIX message: {}", msg_str);

            // Simple parsing for demonstration (real implementation would use proper FIX parsing)
            if msg_str.contains("35=D") {
                // New Order Single
                println!("Processing New Order Single from {}", self.client_id);
                
                // In a real implementation, parse all fields properly
                let cl_ord_id = format!("ORDER_{}", chrono::Utc::now().timestamp_millis());
                let symbol = "EURUSD".to_string();
                let side = '1'; // Buy
                let qty = 1000000;
                let price = Some(1.1234);
                let ord_type = '2'; // Limit

                if let Err(e) = self.order_manager.handle_new_order(
                    cl_ord_id, symbol, side, qty, price, ord_type
                ).await {
                    println!("Order rejected: {}", e);
                }
            } else if msg_str.contains("35=F") {
                // Order Cancel Request
                println!("Processing Order Cancel Request from {}", self.client_id);
                
                // Extract ClOrdID (simplified)
                let cl_ord_id = "ORDER_123".to_string(); // In reality, parse from message
                
                if let Err(e) = self.order_manager.handle_order_cancel(cl_ord_id).await {
                    println!("Cancel rejected: {}", e);
                }
            }
        }
    }

    async fn on_session_disconnect(&mut self, _session: &Session, reason: fixg::DisconnectReason) {
        println!("Trading client {} disconnected: {:?}", self.client_id, reason);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting Order Management System (Artio-style)");

    // Start gateway
    let gateway_config = GatewayConfig {
        bind_address: "0.0.0.0:9877".parse()?,
        ..Default::default()
    };

    let gateway_handle = Gateway::spawn(gateway_config).await?;
    println!("Trading gateway listening on 0.0.0.0:9877");

    // Create trading client
    let client_config = FixClientConfig::new(200);
    let mut client = FixClient::connect(client_config, gateway_handle).await?;

    let session_config = SessionConfig::builder()
        .host("127.0.0.1")
        .port(9877)
        .sender_comp_id("TRADER1")
        .target_comp_id("OMS")
        .heartbeat_interval_secs(30)
        .build()?;

    let session = client.initiate(session_config).await?;
    println!("Trading session established");

    // Create handler and run
    let handler = TradingHandler::new("TRADER1".to_string());
    
    // Simulate sending some orders after a delay
    let session_clone = session.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        
        // Send a new order
        let new_order = "35=D|11=ORDER001|55=EURUSD|54=1|38=1000000|40=2|44=1.1234";
        println!("Sending new order...");
        let _ = session_clone.send_raw(new_order.as_bytes()).await;

        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        
        // Send another order
        let new_order2 = "35=D|11=ORDER002|55=GBPUSD|54=2|38=500000|40=1";
        println!("Sending second order...");
        let _ = session_clone.send_raw(new_order2.as_bytes()).await;
    });

    println!("Order Management System running...");
    println!("Press Ctrl+C to stop");

    client.run(handler).await?;

    Ok(())
}
