# TODO: Roadmap toward DESIGN.md goals

This document tracks the remaining work to evolve `fixg` from the current prototype into the architecture described in `DESIGN.md`.

Status snapshot (current code):
- In-process Gateway and Client with `tokio::mpsc` as the comms channel
- Initiator-only, outbound TCP connect; no acceptor/listener
- Raw payload passthrough, no FIX tag-value parsing/validation
- Minimal `Session` abstraction and callbacks
- Placeholder message types/encoding in `src/messages/`
- Basic typed admin messages generated at build time (`src/messages/generated.rs`); client exposes `Session::send_admin` and inbound `InboundMessage::admin()`

## 0. Foundations and hygiene
- [ ] Establish MSRV and enable CI (fmt, clippy, tests, docs build)
- [ ] Add `cargo deny` and `RUSTFLAGS`/`rustdoc` lint gates (deny warnings)
- [ ] Define coding & API conventions (naming, error handling, docs expectations)
- [ ] Add crate-level docs per module and examples coverage

## 1. FIX Session Protocol (MVP)
Implement minimal FIX 4.x session protocol to support logon→heartbeat/test→logout lifecycle.
- [x] Parser/encoder for tag-value wire format
  - [x] Compute/verify BodyLength (9) and CheckSum (10)
  - [x] Handle SOH delimiter (escaping rules and RawData fields: pending)
  - [x] Strict header/trailer validation (8/9/35/10)
- [x] Session state machine
  - [x] Outbound initiator handshake (Logon, Heartbeat, TestRequest, Logout)
  - [x] Inbound heartbeat/TestRequest response logic
  - [x] Heartbeat interval timers and disconnect timeouts
  - [x] Sequence numbers management (per direction) (minimal: outbound increment, inbound tracking)
- [ ] Minimal persistence of session state (sequence numbers, last logon)
- [x] Example: initiator <-> toy acceptor exchange demonstrating logon/heartbeat/logout

## 2. Acceptor Mode
- [ ] Add TCP listener in `Gateway` (bind to `GatewayConfig.bind_address`)
- [ ] Accept inbound connections and create `Session`s
- [ ] Implement logon negotiation and rejection paths (CompIDs, heartbeat interval, reset)
- [ ] Pluggable authentication hook on logon (strategy in `GatewayConfig`)

## 3. Message Codec Generation
Generate zero-copy codecs from FIX XML dictionaries, replacing placeholders in `src/messages/`.
- [ ] New crate: `fixg-codegen`
- [ ] Parse FIX/FIXT XML dictionaries
- [ ] Generate Rust types for fields/components/messages
- [ ] Outbound builders that produce `bytes::Bytes` without intermediate allocations
- [ ] Inbound decoders over `&[u8]`/`Bytes` with zero-copy field views
- [x] Build integration via `build.rs` or standalone `codegen` binary
- [x] Initial generated admin messages and typed conversions (`build.rs` -> `src/messages/generated.rs`)
- [ ] Validation constraints per dictionary (required fields, value ranges)
- [ ] Property-based tests with golden messages (round-trip encode/decode)
- [ ] Replace `src/messages/*` placeholders with generated code

## 4. Persistence and Replay
Satisfy resend/replay requirements and recovery after restarts.
- [ ] Define storage abstraction (trait) for message journaling and indexing
- [ ] Implement a file-backed append-only log with an index for quick lookup
- [ ] Persist inbound/outbound messages with sequence numbers and timestamps
- [ ] Implement ResendRequest/SequenceReset (GapFill and Fill) flows
- [ ] Recovery on restart (hydrate session state and seq nums)
- [ ] End-of-day logic (logout/reset policies)

## 5. Engine↔Library transport
Separate concerns and prepare for local IPC and remote UDP.
- [ ] Define internal wire protocol between Gateway and Client (versioned messages)
- [ ] Keep in-process `tokio::mpsc` backend for dev profile
- [ ] Abstract transport with a trait to support Aeron later (feature flag)
- [ ] Backpressure semantics and bounded queues; drop/linger policies

## 6. Aeron Integration
Adopt Aeron channels for IPC/UDP and enable distributed deployment.
- [ ] Evaluate `aeron-rs` vs. FFI binding (licensing, maintenance, performance)
- [ ] Implement Publication/Subscription mapping to Engine↔Library protocol
- [ ] Configure channels (ipc vs udp?endpoint=host:port) via `GatewayConfig`
- [ ] Handle backpressure (offer/poll) and idle strategies
- [ ] Diagnostic tooling: counters, stream inspection

## 7. High Availability (Aeron Cluster)
- [ ] Integrate Gateway with Aeron Cluster client for replicated command log
- [ ] Fast failover: followers hydrate state and local archive
- [ ] Library reconnection and session re-acquisition after leader change
- [ ] Idempotency and duplicate suppression on resend/replay

## 8. API ergonomics and correctness
- [ ] Enforce single-threaded `FixClient` semantics (`!Send`/`!Sync` where needed)
- [ ] Current-thread runtime option and guidance for latency-sensitive apps
- [ ] Explicit backpressure APIs and error surfaces for `Session::send`
- [ ] Typed message APIs using generated codecs; deprecate raw payload paths from hot code
- [x] Basic typed admin message APIs: `Session::send_admin`, inbound `InboundMessage::admin()`
- [ ] Rich error types (`thiserror`) and actionable diagnostics

## 9. Security and TLS
- [ ] Optional TLS for transport using `tokio-rustls`
- [ ] Credential handling for Logon (username/password) and custom validation
- [ ] Configurable cipher suites and cert reloading

## 10. Observability
- [ ] `tracing` spans around I/O, parsing, session state transitions
- [ ] Metrics (Prometheus): latencies, queue depths, drops, resend counts
- [ ] Per-session instrumentation and sampling
- [ ] Structured logs with message IDs and sequence numbers

## 11. Testing and Quality
- [ ] Unit tests for parsing, encoding, state machine transitions
- [ ] Integration tests with real FIX message fixtures
- [ ] Fuzzers for parser and state machine (libFuzzer/AFL)
- [ ] Long-running soak tests (disconnects, resends, fault injection)
- [ ] Conformance tests against a reference counterparty (e.g., a public FIX server or harness)

## 12. Performance and Benchmarks
- [ ] Criterion microbenchmarks for codecs and session loops
- [ ] End-to-end latency/throughput benchmarks (p50/p99/p99.9)
- [ ] Zero-allocation on hot paths; track allocations with instrumentation
- [ ] Tune channel sizes, OS buffers, and runtime settings; document guidance

## 13. Documentation and Examples
- [ ] Expand crate docs with a guided tutorial and architecture overview
- [ ] Examples: acceptor, initiator, echo, order flow, market data
- [ ] Design record of tradeoffs (Aeron vs in-house, storage formats)
- [ ] Publish docs (docs.rs readiness and optional GitHub Pages)

## 14. Release and Packaging
- [ ] Versioning policy and release checklist
- [ ] Changelog and upgrade notes
- [ ] License files and headers verification

---

### Suggested Milestones
- M1: Session MVP (1,2) + Example initiator/acceptor demo
- M2: Generated codecs (3) + Typed message send/receive — initial admin-only support implemented; full dictionary-based codegen pending
- M3: Persistence/replay (4) + Robust resend handling
- M4: Engine↔Library abstraction (5) + Aeron prototype (6)
- M5: HA with Aeron Cluster (7) + hardening (8–12)

### Open Questions/Risks
- Aeron integration path and maintenance cost
- FIX version coverage scope (4.2/4.4/FIXT 1.1/5.0SP2)
- Storage format (custom vs. interoperable) and ops tooling
- Allocation budgets and zero-copy boundaries across modules

## M2→M3 Hardening Plan (appended)

To evolve from the initial typed admin messages and minimal journaling/resend into the robust architecture in `DESIGN.md`, target the following increments:

- Storage robustness
  - Define a pluggable `MessageStore` trait with backpressure-aware write path and batching.
  - Provide fsync and durability policies (always, interval, disabled) with metrics.
  - Introduce per-session rotating segments with index files for O(log n) lookups; memory-map indexes.
  - Add corruption detection and recovery tooling (checksum, verifier CLI, truncate/repair on startup).
  - Support compaction and archival; document retention policies and disk quotas.

- Sequence numbers and recovery
  - Persist last known seq numbers and last logon state; reload on restart.
  - Implement full resend semantics including handling 16=0 (infinite) and chunked replays.
  - Implement SequenceReset GapFill and Fill flows with correct range accounting and idempotency.
  - Detect and suppress duplicates during replay using journal index.

- Protocol correctness
  - Enforce sending times (52) and OrigSendingTime (122) on resends; validate PossDupFlag (43) and PossResend (97).
  - Validate required headers per dictionary; configurable BeginString and ApplVerID where applicable.
  - Clock skew handling and optional time-source abstraction.

- Typed codecs (beyond admin)
  - Introduce `fixg-codegen` crate: parse FIX XML, generate fields, components, messages.
  - Zero-copy decoding over `&[u8]` using field views; avoid allocations in hot path.
  - Outbound builders produce `Bytes` directly with pre-sized buffers and tag order policy.
  - Validation constraints from dictionary (Reqd, Enums, min/max) with helpful errors.
  - Property-based testing with golden fixtures; fuzz decoding.

- Engine↔Library contract
  - Formalize internal wire protocol for events/commands; version it and add feature-gated transports.
  - Specify backpressure semantics; introduce bounded queues and observable drops.
  - Add idle strategies and cooperative scheduling guidance.

- Observability
  - Add `tracing` spans for I/O, parsing, session transitions, storage ops; include seq nums and session keys.
  - Export Prometheus metrics: queue depths, resend counts, duplicates suppressed, write latencies, fsync behavior.
  - Structured logs with message IDs and correlation to journal offsets.

- Testing matrix
  - Unit tests: parser, encoding, state machine, storage index.
  - Integration tests: initiator/acceptor with fixture dictionaries; restart/recovery scenarios.
  - Soak and fault-injection: network partitions, delayed packets, disk full, corruption.
  - Conformance tests against public FIX endpoints or a harness.

- Performance
  - Benchmarks for encode/decode, session loop, journaling (batched writes vs fsync), resend throughput.
  - Lock-free or minimal-lock designs; tune channel sizes and OS buffers.
  - Allocation tracking and zero-allocation goals on the hot path.

- API ergonomics
  - Typed application messages; deprecate raw payloads in hot code paths.
  - Clear error types (`thiserror`), backpressure-aware send API, and current-thread runtime option.
  - Document best practices for latency-sensitive apps.