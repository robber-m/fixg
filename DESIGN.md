## Deconstructing Artio: A Blueprint for a High-Performance FIX Engine in Rust

### Overview
This document distills the architectural principles of Real Logic’s Artio and adapts them into a concrete plan for a Rust implementation named **fixg**. It emphasizes the engine/library split, Aeron-backed messaging and persistence, single-threaded event-driven application logic, and code generation for zero-allocation message codecs. It also proposes an idiomatic Rust API leveraging async/await, ownership, and the `bytes` crate.

---

## Section 1: The Artio Architectural Blueprint: A Dichotomy of Engine and Library

Designing a high-performance FIX engine requires clear separation of concerns, predictable latency, and operational resilience. Artio embodies a proven “core/satellite” model that cleanly divides protocol mechanics from application logic. This separation is foundational to fixg.

### 1.1 Introduction to the Core/Satellite Model
- **Core (Engine)**: Handles TCP sockets, FIX session protocol (logon, heartbeat, sequence numbers), persistence and replay, and framing bytes into FIX messages.
- **Satellite (Library)**: Presents an API for business logic to send/receive application messages, delegating transport/session management to the core.

Artio implements this as `FixEngine` (core) and one or more `FixLibrary` (satellites). fixg adopts this model to decouple I/O-critical responsibilities from application-specific logic.

### 1.2 The FixEngine: The System’s Resilient Core
- **Connectivity and Framing**: Accepts/initiates TCP connections, frames inbound byte streams into messages, performs basic validation.
- **Durability and Replay**: Archives all FIX streams using Aeron Archive to satisfy resend requests and enable recovery.
- **Session Authority**: Owns session state; libraries explicitly claim session ownership. If no library is present, the Engine maintains protocol liveness (e.g., heartbeats).
- **Lifecycle Orchestration**: End-of-day logout, sequence resets, administrative controls.

### 1.3 The FixLibrary: The Application’s Gateway to FIX
- **Single-threaded Reactor**: Not thread-safe; progressed via a poll/duty cycle. Eliminates locks for predictable latency.
- **Asynchrony by Design**: Operations return immediately (e.g., Reply objects) and complete via polling in the same duty cycle.
- **Application Interface**: Initiates/claims/releases sessions, parses inbound messages, constructs outbound payloads.

### 1.4 Deployment Flexibility: Co-located vs. Distributed
- **IPC or UDP**: Engine and Library can be co-located (shared memory IPC) or distributed (UDP) using Aeron channels.
- **Centralized Engine, Multiple Libraries**: One Engine servicing many Library processes across hosts.

For fixg, the split maps naturally to Rust:
- **Gateway (Engine)**: A long-lived async task managing network I/O via `tokio::net::TcpListener`, framing, session protocol, and persistence.
- **FixClient (Library)**: A single-threaded, event-driven API for application logic. Enforce `!Send`/`!Sync` where appropriate to preserve the concurrency model.
- **Comms Channel**: In-process via `tokio::mpsc`; inter-process via Aeron (FFI or `aeron-rs`).
- **Current-thread Runtime**: Use `tokio::runtime::Builder::new_current_thread()` for lowest latency when desirable.

---

## Section 2: The Aeron Messaging Fabric: Artio’s High-Speed Nervous System

Artio’s Engine↔Library separation is enabled by Aeron, providing transport, persistence, and consensus (via Aeron Cluster).

### 2.1 Aeron as the Transport Layer
- **Configuration**: Engine and Library specify Aeron channel strings (e.g., `aeron:ipc` for shared memory, `aeron:udp?endpoint=host:port` for UDP).
- **Streams**: Stream IDs multiplex logical data flows on a single channel.
- **Transparency**: Switching between IPC and UDP requires no application logic changes.

### 2.2 Aeron Archive for Persistence and Replay
- **Durable Log**: Append-only, indexed logs for resend and recovery.
- **Replay Controls**: Tunables such as `replayIndexFileRecordCapacity` affect replay efficiency windows.
- **Tooling**: Archive inspection and audit utilities.

### 2.3 Aeron Cluster for High Availability (HA)
- **Replicated State Machine**: Raft-based leader/followers with fast failover; supports Active/Passive and Active/Active.
- **Follower APIs**: Followers hydrate state and local archive to be takeover-ready.

Implications for fixg:
- **Build vs. Integrate**: Either integrate Aeron via FFI/`aeron-rs` (preferred), or reimplement transport/persistence/consensus in Rust (high effort and risk).
- **Programming Model**: Aeron’s poll-driven Publication/Subscription aligns with a non-blocking, single-threaded event loop. In Rust, expose an async/await façade returning `Future`s instead of pollable `Reply` objects.

---

## Section 3: Mechanical Sympathy in Practice: The “Real Logic” Low-Latency Philosophy

### 3.1 The Enemy: GC Pauses
GC introduces non-deterministic pauses unacceptable for microsecond-scale latencies. Artio avoids heap allocations on hot paths to minimize GC impact.

### 3.2 The Solution: Agrona and Off-Heap Memory
- **Off-heap Buffers**: `DirectBuffer`, `MutableDirectBuffer`, `AtomicBuffer` to bypass GC.
- **Primitive Collections**: Avoid boxing overhead.

### 3.3 Code Generation for Zero-Allocation Codecs
- **Generated Encoders/Decoders**: From FIX or SBE schemas; operate directly on buffers without allocations.

Rust perspective for fixg:
- **No GC**: Rust’s ownership model makes the GC-avoidance “sandbox” the default.
- **Buffers**: Use `bytes::Bytes` and `BytesMut` for zero-copy networking.
- **Collections**: `HashMap` has no boxing for primitives.
- **Code Generation is Essential**: Generate efficient, zero-copy encoders/decoders from FIX XML at build time (via `build.rs` or procedural macros).

---

## Section 4: Bridging the Chasm: Translating High-Performance Java to Idiomatic Rust

### 4.1 Performance Philosophies: JIT vs. AOT
- **Java (JIT)**: Hot-spot optimizations, warmup, GC management.
- **Rust (AOT)**: Predictable from first run; zero-cost abstractions; compile-time safety.

### 4.2 Abstraction Differences: Will they be different? Yes.
- **Memory**: Agrona buffers vs. Rust’s ownership, `Vec<u8>`, and `bytes`.
- **Concurrency**: Single-threaded library in Artio vs. Rust’s `Send`/`Sync`-checked concurrency; still enforce single-threaded semantics where needed for predictability.
- **Error Handling**: Exceptions/Reply vs. `Result<T, E>`/`Option<T>` and async `Future`s.

### 4.3 A Conceptual Mapping
- **Buffer Ownership**: Prefer APIs that take ownership (e.g., `Bytes`) to leverage move semantics and prevent double-use.
- **Ecosystem Composition**: Combine `tokio`, `bytes`, optional `aeron-rs`, `serde`, and generated codecs into a cohesive design.

#### Table 1: Java (Artio) to Rust (fixg) Concept Mapping
| Artio (Java) Concept | fixg (Rust) Equivalent | Rationale for Translation |
|---|---|---|
| FixEngine Process | Gateway (async task/actor) | Encapsulates core I/O and persistence, mirroring FixEngine’s role as a separate process. |
| FixLibrary Instance | FixClient (struct, likely `!Send` & `!Sync`) | Enforces single-threaded event loop, translating Artio’s concurrency model. |
| LibraryConfiguration, EngineConfiguration | `GatewayConfig`, `FixClientConfig` (structs) | Type-safe configuration; integrate with `serde` for TOML/JSON. |
| `FixLibrary.poll()` | `FixClient::run(handler)` (async fn) | Rust async loop replaces manual polling for a cleaner API. |
| Session Class | `Session` (struct) | Same concept; provides methods to interact and send messages. |
| `Session.trySend(buffer, offset, len)` | `Session::try_send(&self, payload: Bytes)` | Moves ownership of the buffer to prevent accidental reuse. |
| Agrona `DirectBuffer` | `bytes::Bytes`, `bytes::BytesMut` | Idiomatic zero-copy buffers for networking in Rust. |
| Agrona `Int2IntHashMap` | `std::collections::HashMap<i32, i32>` | No boxing overhead for primitive keys/values. |
| Aeron IPC/UDP Channel | `tokio::mpsc::channel` or `aeron-rs` | Tokio MPSC for in-process; Aeron for inter-process/HA. |
| Generated Encoder/Decoder Classes | Generated Encoder/Decoder Traits | Traits implemented by per-message generated structs.

---

## Section 5: Designing ‘fixg’: A Proposed API and Documentation Outline

### 5.1 Crate Overview and Core Concepts (`lib.rs`)
Welcome to **fixg**, a high-performance, asynchronous toolkit for building FIX protocol applications in Rust. fixg draws inspiration from Artio’s architecture while embracing idiomatic Rust for safety and ergonomics.

- **Gateway**: The I/O engine of fixg. Runs as a background task, managing TCP connections, framing, session protocol, and persistence.
- **FixClient**: Your application’s handle to the Gateway. Designed to be driven by a single-threaded event loop.

#### A “Hello, World!” Initiator
```rust
use fixg::{Gateway, GatewayConfig, FixClient, FixClientConfig, FixHandler, Session, InboundMessage};
use fixg::session::SessionConfig;
use fixg::messages::Logon; // Generated message type (placeholder)
use async_trait::async_trait;

struct MyApp;

#[async_trait]
impl FixHandler for MyApp {
    async fn on_session_active(&mut self, session: &Session) {
        println!("Session is now active! Session ID: {}", session.id());
    }

    async fn on_message(&mut self, _session: &Session, msg: InboundMessage<'_>) {
        println!("Received message with type: {}", msg.msg_type());
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Configure and spawn the Gateway
    let gateway_config = GatewayConfig::default();
    let gateway_handle = Gateway::spawn(gateway_config).await?;

    // 2. Configure and connect the FixClient
    let client_config = FixClientConfig::new(1); // Library ID 1
    let mut client = FixClient::connect(client_config, gateway_handle).await?;

    // 3. Configure and initiate a session
    let session_config = SessionConfig::builder()
        .host("127.0.0.1")
        .port(9876)
        .sender_comp_id("INITIATOR")
        .target_comp_id("ACCEPTOR")
        .build()?;
    let _session = client.initiate(session_config).await?;

    // 4. Run the event loop
    println!("Client running, waiting for events...");
    client.run(&mut MyApp).await?;

    Ok(())
}
```

### 5.2 The Gateway and FixClient Lifecycle

#### Configuration
Use type-safe structs (serde-friendly) to mirror Artio’s key parameters: networking, persistence, and behavior.

#### Table 2: fixg Gateway and FixClient Configuration Parameters
| Parameter | Component | Type | Default | Description & Impact |
|---|---|---|---|---|
| `log_directory` | Gateway | `PathBuf` | `"./fixg_logs/"` | Directory for Aeron Archive logs and other persistent state. Critical for message recovery. |
| `aeron_channel` | Gateway | `String` | `"aeron:ipc"` | Aeron channel for Gateway↔Client communication. Use `ipc` for same-machine, `udp?...` for network. |
| `bind_address` | Gateway | `SocketAddr` | `0.0.0.0:4050` | Listening address and port for acceptor connections. |
| `authentication_strategy` | Gateway | `Box<dyn AuthStrategy>` | `AcceptAll` | Pluggable authentication on logon (validate CompIDs/credentials). |
| `library_id` | Client | `i32` | (Required) | Unique identifier for this FixClient instance. |
| `dictionaries` | Client | `Vec<FixDictionary>` | `` | FIX XML dictionaries to load for codec generation. |
| `async_runtime` | Both | `enum { CurrentThread, MultiThread }` | `MultiThread` | Choose current-thread for lowest latency; multithread for mixed workloads. |

#### Initialization and Event Loop
- **Spawn Gateway**: `Gateway::spawn(config)` starts the I/O engine and returns a `GatewayHandle`.
- **Connect Client**: `FixClient::connect(config, handle)` sets up communication with the Gateway.
- **Run Event Loop**: `client.run(&mut handler).await` drives application logic; returns on disconnect or fatal error.

### 5.3 Handling FIX Sessions and Business Logic

#### The `FixHandler` Trait
```rust
use async_trait::async_trait;
use fixg::{Session, InboundMessage, DisconnectReason};

#[async_trait]
pub trait FixHandler {
    /// Called when a new inbound message arrives for a session.
    /// The `msg` parameter is a zero-copy view over the receive buffer.
    async fn on_message(&mut self, session: &Session, msg: InboundMessage<'_>);

    /// Called when a session's logon is complete and it is fully active.
    async fn on_session_active(&mut self, session: &Session);

    /// Called when a session is disconnected from its counterparty.
    async fn on_disconnect(&mut self, session: &Session, reason: DisconnectReason);
}
```

#### Session Management
- `client.initiate(session_config).await? -> Session`: Initiates a new outbound connection and logon; becomes active after handshake (`on_session_active`).
- `Session::send(msg).await?`: Primary method for sending encoded FIX messages.

### 5.4 Zero-Copy Message Handling: The Generated Codecs

fixg provides a code generation utility (e.g., `fixg-codegen`) to transform FIX XML dictionaries into efficient Rust code.

- **Outbound**: Builder-style structs per message type (e.g., `NewOrderSingle`).
- **Inbound**: Zero-copy decoders operating on `bytes::Bytes` views (e.g., `NewOrderSingleDecoder`).

#### Parsing an Inbound Message
```rust
// Inside on_message(..., msg: InboundMessage<'_>)
match msg.msg_type() {
    "D" => { // NewOrderSingle
        if let Ok(nos_decoder) = NewOrderSingleDecoder::try_from(msg.body()) {
            let cl_ord_id = nos_decoder.cl_ord_id()?;
            let price = nos_decoder.price()?;
            println!("Received new order {} at price {}", cl_ord_id, price);
        }
    },
    _ => { /* handle other message types */ }
}
```

#### Encoding and Sending an Outbound Message
```rust
let exec_report = ExecutionReport::builder()
    .cl_ord_id("order-123")
    .order_id("exec-456")
    .exec_type(ExecType::Fill)
    .ord_status(OrdStatus::Filled)
    .last_px(150.25)
    .last_qty(100)
    .build();

session.send(exec_report.into()).await?;
```

### 5.5 Advanced Patterns
- **Session Metadata**: Attach application-specific, type-safe metadata to `Session` (extensions map pattern).
- **High Availability**: Integrate Gateway with an Aeron Cluster client (`aeron-rs`), keeping FixClient interaction unchanged while underlying events flow through a replicated log.

---

## Section 6: Conclusion and Strategic Recommendations

### 6.1 Summary of Lessons from Artio
- **Engine/Library Split**: Decouple I/O from business logic for independent optimization and scaling.
- **Specialized Fabric**: Delegate transport, persistence, and consensus to Aeron.
- **Mechanical Sympathy**: Eliminate unpredictable latency on the hot path through careful memory and API design.

### 6.2 The ‘fixg’ Advantage
- **Compile-time Safety**: Ownership and `Send`/`Sync` enable fearless concurrency with no data races.
- **Performance by Default**: Zero-cost abstractions, no warmup, no GC pauses.
- **Ergonomic Async**: `async`/`await`, `Result`, and `Option` provide explicit and robust APIs.

### 6.3 Key Challenges and Recommendations
- **Challenge: Codec Generator**
  - Recommendation: Highest priority. Implement as `build.rs` or procedural macros. Target zero-copy buffers (`bytes`).
- **Challenge: Aeron Dependency**
  - Recommendation: Favor integrating `aeron-rs` (or FFI) after rigorous evaluation. Avoid re-implementing Aeron unless absolutely necessary.
- **Challenge: API Ergonomics**
  - Recommendation: Embrace Rust idioms. Design APIs around ownership/borrowing and async Futures rather than direct Java-style polling.

---

## Appendix: Notes on Concurrency Discipline in fixg
- Prefer a single-threaded `FixClient` (`!Send`/`!Sync`) progressing on a current-thread runtime for predictable latency.
- Use bounded `tokio::mpsc` channels between `FixClient` and Gateway to maintain backpressure.
- Isolate CPU-bound work off the event loop using dedicated tasks and message passing.
- Provide instrumentation (histograms, counters) for duty-cycle duration, queue depths, and replay latencies.