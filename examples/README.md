
# fixg Examples - Artio Architecture Validation

This directory contains examples that demonstrate how `fixg` implements the key architectural principles from Real Logic's Artio FIX engine.

## Examples Overview

### Core Architecture Examples

1. **`artio_validation_test.rs`** - Comprehensive validation of Artio principles
   - Engine/Library separation
   - Multiple concurrent Library instances
   - Message ordering and delivery guarantees
   - Performance characteristics validation

2. **`market_data_gateway.rs`** - High-frequency market data distribution
   - Multi-client market data distribution
   - Broadcast messaging patterns
   - Session lifecycle management
   - Backpressure handling

3. **`order_management.rs`** - Complete order lifecycle management
   - Order entry and validation
   - Execution reporting
   - State persistence
   - Risk management patterns

### Running the Examples

Each example can be run independently:

```bash
# Validate core Artio architectural compliance
cargo run --example artio_validation_test

# Market data gateway with multiple clients
cargo run --example market_data_gateway

# Order management system
cargo run --example order_management
```

### Aeron Integration Examples

When the `aeron-ffi` feature is enabled, examples will use Aeron for transport and persistence:

```bash
# Run with Aeron integration
cargo run --features aeron-ffi --example artio_validation_test
```

## Architecture Validation

### Engine/Library Separation

The examples demonstrate clear separation between:
- **Gateway (Engine)**: Network I/O, session protocol, persistence
- **FixClient (Library)**: Application logic, message handling, business rules

### Performance Characteristics

Expected performance (similar to Artio):
- **Latency**: Sub-millisecond round-trip times
- **Throughput**: 100K+ messages/second per session
- **Memory**: Zero allocations on hot message paths
- **CPU**: Single-threaded event loops for predictable latency

### Session Management

All examples demonstrate:
- Proper session lifecycle (logon, heartbeat, logout)
- Message sequencing and gap detection
- Administrative message handling
- Graceful disconnect handling

## Artio Design Patterns Implemented

### 1. Reactor Pattern
- Single-threaded event loops in Library instances
- Non-blocking I/O with async/await
- Event-driven message processing

### 2. Command-Query Separation
- Commands flow from Library to Engine (session control, message sends)
- Events flow from Engine to Library (session events, inbound messages)

### 3. Message Ordering
- Strict FIFO ordering within sessions
- Sequence number validation
- Gap detection and resend requests

### 4. Resource Management
- RAII for session cleanup
- Graceful resource release on disconnect
- Memory pool patterns for message buffers

## Comparison with Artio

| Aspect | Artio (Java) | fixg (Rust) |
|--------|-------------|------------|
| Engine/Library Split | ✓ | ✓ |
| Aeron Integration | ✓ | ✓ (feature-gated) |
| Zero-copy Messaging | ✓ | ✓ (via bytes crate) |
| Single-threaded Library | ✓ | ✓ (enforced by design) |
| Message Codecs | Generated | Generated (planned) |
| Archive Integration | ✓ | ✓ (in progress) |
| High Availability | ✓ (Cluster) | Planned |

## Next Steps

1. **Full FIX Dictionary Support**: Complete message codec generation
2. **Aeron Archive Integration**: Full replay and recovery capabilities
3. **Cluster Support**: HA deployment with leader election
4. **Performance Optimization**: Profile and optimize hot paths
5. **Conformance Testing**: Validate against FIX protocol test suites

## Performance Tuning

For optimal performance similar to Artio:

1. **OS Configuration**:
   ```bash
   # CPU isolation
   echo 'isolcpus=1,2,3' >> /boot/cmdline.txt
   
   # Memory settings
   echo 'vm.swappiness=1' >> /etc/sysctl.conf
   ```

2. **Runtime Configuration**:
   ```rust
   // Use current-thread runtime for lowest latency
   let rt = tokio::runtime::Builder::new_current_thread()
       .enable_all()
       .build()?;
   ```

3. **Aeron Configuration** (when using `aeron-ffi`):
   ```properties
   aeron.threading.mode=DEDICATED
   aeron.conductor.cpu.affinity=0
   aeron.sender.cpu.affinity=1
   aeron.receiver.cpu.affinity=2
   ```

These examples serve as both validation tools and reference implementations for building high-performance FIX applications with `fixg`.
