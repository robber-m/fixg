use fixg::{Gateway, GatewayConfig, FixClient, FixClientConfig, FixHandler, Session, InboundMessage};
use fixg::session::SessionConfig;
use async_trait::async_trait;

struct MyApp;

#[async_trait]
impl FixHandler for MyApp {
    async fn on_session_active(&mut self, session: &Session) {
        println!("Session is now active! Session ID: {}", session.id());
    }

    async fn on_message(&mut self, _session: &Session, msg: InboundMessage) {
        println!("Received message with type: {}", msg.msg_type());
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gateway_config = GatewayConfig::default();
    let gateway_handle = Gateway::spawn(gateway_config).await?;

    let client_config = FixClientConfig::new(1);
    let mut client = FixClient::connect(client_config, gateway_handle).await?;

    let session_config = SessionConfig::builder()
        .host("127.0.0.1")
        .port(9876)
        .sender_comp_id("INITIATOR")
        .target_comp_id("ACCEPTOR")
        .build()?;

    let _session = client.initiate(session_config).await?;

    println!("Client running, waiting for events...");
    client.run(&mut MyApp).await?;
    Ok(())
}