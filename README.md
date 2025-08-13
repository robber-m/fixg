# fixg

A high-performance, async FIX engine and client library in Rust.

Status: early prototype. Networking, session scaffolding, and an initiator example exist. FIX protocol parsing/validation, logon/heartbeat handling, persistence, Aeron integration, and production-ready features are not yet implemented.

## Features (current)
- Gateway task that manages client registrations and TCP sessions
- Client API to connect to the gateway and initiate a TCP session to a host:port
- Simple event callbacks: `on_session_active`, `on_message`, `on_disconnect`
- Send and receive raw payloads via `bytes::Bytes`

## Not yet implemented
- FIX message parsing/validation and session protocol (Logon/Heartbeat/SeqNums)
- Persistence/replay, Aeron IPC/UDP, HA/cluster
- Acceptor mode, authentication hooks
- Code generation for FIX codecs

## Quickstart
Requires latest stable Rust.

- Build and run the initiator example:

```bash
cargo run --example initiator
```

- Minimal program using the library API:

```rust
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
    let gateway_handle = Gateway::spawn(GatewayConfig::default()).await?;

    let mut client = FixClient::connect(FixClientConfig::new(1), gateway_handle).await?;

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
```

## API docs (Rustdoc)
- Generate and open local docs:

```bash
cargo doc --no-deps --open
```

The crate’s docs include this README at the crate root.

## Project layout
- `src/gateway.rs`: Gateway task and handle
- `src/client.rs`: Client API and event loop
- `src/session.rs`: Session type and configuration builder
- `src/config.rs`: Config types
- `src/messages/`: Placeholder for generated and hand-written message types
- `examples/initiator.rs`: Minimal initiator wiring
- `DESIGN.md`: Architectural notes and roadmap

## License
Apache-2.0 OR MIT
