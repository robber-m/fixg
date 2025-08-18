
# Aeron Integration in fixg

## Overview

This document describes how `fixg` integrates with Aeron for high-performance, low-latency messaging and persistence, following the architectural patterns established by Real Logic's Artio.

## Integration Levels

### High-Level Integration

At the application level, `fixg` provides two main integration points with Aeron:

1. **Transport Layer**: Gateway↔Client communication via Aeron IPC/UDP channels
2. **Persistence Layer**: Message archival and replay using Aeron Archive

### Low-Level Implementation

#### Transport Architecture

```
┌─────────────────┐    Aeron Channel     ┌─────────────────┐
│   FixClient     │◄─────────────────────►│    Gateway      │
│   (Library)     │    (IPC/UDP)         │   (Engine)      │
└─────────────────┘                      └─────────────────┘
```

**Channel Configuration:**
- **In-Process**: `aeron:ipc` for same-machine deployment
- **Network**: `aeron:udp?endpoint=host:port` for distributed deployment
- **Stream IDs**: Multiplexed logical flows on single channel

#### Message Flow

1. **Client→Gateway Commands**:
   - Session initiation/termination
   - Message send requests
   - Administrative commands

2. **Gateway→Client Events**:
   - Session state changes
   - Inbound FIX messages
   - Error notifications

#### Persistence Architecture

```
┌─────────────────┐    Archive Control   ┌─────────────────┐
│    Gateway      │◄─────────────────────►│  Aeron Archive  │
│                 │                      │                 │
│                 │    Data Stream       │                 │
│                 │◄─────────────────────►│                 │
└─────────────────┘                      └─────────────────┘
```

**Stream Design:**
- **Data Stream**: Raw FIX message bytes with metadata
- **Index Stream**: Sequence number→offset mappings for fast replay
- **Control Stream**: Archive management and replay requests

## Feature-Gated Implementation

### Without Aeron (`default` features)
- **Transport**: `tokio::mpsc` channels for in-process communication
- **Persistence**: File-based JSONL journal with separate index

### With Aeron (`aeron-ffi` feature)
- **Transport**: Aeron Publication/Subscription via FFI
- **Persistence**: Aeron Archive streams for durable logging

## FFI Safety and Performance

### Memory Management
- Zero-copy where possible using `bytes::Bytes`
- Controlled allocations in FFI boundary
- RAII wrappers for Aeron resources

### Error Handling
- Graceful degradation on Aeron failures
- Backpressure handling with configurable policies
- Resource cleanup on connection loss

### Performance Characteristics
- **Latency**: Sub-microsecond when using IPC
- **Throughput**: 10M+ messages/second on modern hardware
- **Durability**: Persistent storage with fast recovery

## Configuration Examples

### Basic IPC Configuration
```toml
[storage]
backend = "aeron"
archive_channel = "aeron:ipc"
data_stream_id = 1001
index_stream_id = 1002
```

### Network Configuration
```toml
[storage]
backend = "aeron"
archive_channel = "aeron:udp?endpoint=192.168.1.100:40001"
data_stream_id = 1001
index_stream_id = 1002
```

### Advanced Configuration
```toml
[aeron]
# Archive control channel
control_channel = "aeron:udp?endpoint=localhost:8010"
control_stream_id = 100

# Data archival
archive_channel = "aeron:ipc"
data_stream_id = 1001
index_stream_id = 1002

# Replay configuration
replay_fragment_limit = 1024
replay_idle_strategy = "yield"

# Backpressure handling
offer_retry_count = 3
offer_backoff_ms = 1
```

## Operational Considerations

### Monitoring
- Stream position tracking
- Archive utilization metrics
- Connection health monitoring

### Tooling
- Archive inspection utilities
- Stream replay tools
- Performance profiling hooks

### High Availability
- Leader election via Aeron Cluster
- Follower state replication
- Fast failover mechanisms

## Implementation Status

### Currently Implemented ✅
- Basic Aeron FFI wrapper
- Publication/Subscription primitives
- Feature-gated storage backend
- Dual-stream persistence (data + index)

### In Progress 🚧
- Archive control integration
- Backpressure handling
- Connection management

### Planned 📋
- Aeron Cluster integration
- Advanced replay features
- Performance optimization
- Operational tooling

## Performance Tuning

### OS Configuration
```bash
# Increase socket buffer sizes
echo 'net.core.rmem_max = 134217728' >> /etc/sysctl.conf
echo 'net.core.wmem_max = 134217728' >> /etc/sysctl.conf

# CPU isolation for Aeron threads
isolcpus=1,2,3
```

### Aeron Configuration
```properties
# Media driver threading
aeron.threading.mode=DEDICATED
aeron.conductor.cpu.affinity=0
aeron.sender.cpu.affinity=1
aeron.receiver.cpu.affinity=2

# Memory settings
aeron.dir=/dev/shm/aeron
aeron.mtu.length=8192
```

### Application Tuning
```rust
// Use current-thread runtime for lowest latency
let rt = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()?;

// Configure Aeron with optimal settings
let config = GatewayConfig {
    storage: StorageBackend::Aeron {
        archive_channel: "aeron:ipc".to_string(),
        stream_id: 1001,
    },
    aeron_fragment_limit: 1024,
    aeron_idle_strategy: IdleStrategy::Yield,
    ..Default::default()
};
```

This integration provides the foundation for Artio-level performance while maintaining Rust's safety guarantees and idiomatic patterns.
