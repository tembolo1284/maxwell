# Maxwell Crate Reference

## 1. Purpose

This document describes the responsibilities, ownership
boundaries, and principal invariants of each Maxwell
workspace package.

The workspace contains ten foundational library crates
and four applications.

Public Rust interfaces and Cargo manifests remain the
authoritative references for exact signatures and
compile-time dependencies.

## 2. Workspace Directory

| Package | Location | Responsibility |
|---------|----------|----------------|
| maxwell-core | crates/maxwell-core | Financial domain primitives |
| maxwell-protocol | crates/maxwell-protocol | Binary protocol |
| maxwell-sequencer | crates/maxwell-sequencer | Sequence validation |
| maxwell-book | crates/maxwell-book | Market reconstruction |
| maxwell-transport | crates/maxwell-transport | Communication primitives |
| maxwell-recovery | crates/maxwell-recovery | Recovery coordination |
| maxwell-publisher | crates/maxwell-publisher | Market data distribution |
| maxwell-gateway | crates/maxwell-gateway | Downstream sessions |
| maxwell-client | crates/maxwell-client | Consumer market state |
| maxwell-metrics | crates/maxwell-metrics | Instrumentation |
| maxwell-server | apps/maxwell-server | Integrated server |
| maxwell-simulator | apps/maxwell-simulator | Synthetic market generation |
| maxwell-replay | apps/maxwell-replay | Recording and replay |
| maxwell-cli | apps/maxwell-cli | Operational CLI |

## 3. maxwell-core

### Responsibility

Defines shared financial and market data domain types.

### Capabilities

- Instrument identifiers and definitions.
- Order and trade identifiers.
- Fixed-point prices and quantities.
- Feed, stream, and session identifiers.
- Sequence numbers.
- Exchange and monotonic timestamps.
- Market event representations.
- Event envelopes.

### Invariants

- Financial values must preserve domain constraints.
- Identifier types must not be accidentally interchanged.
- Arithmetic must handle invalid values and overflow.
- Domain types must remain independent of networking,
  persistence, and application orchestration.

### Ownership

Domain values rather than mutable application state.

## 4. maxwell-protocol

### Responsibility

Defines Maxwell's native binary message representation.

### Capabilities

- Versioned protocol headers.
- Message-kind identification.
- Binary encoding and decoding.
- Message-length validation.
- Event payload serialization.
- Caller-provided encoding buffers.

### Invariants

- Wire representations must be explicit.
- Rust memory layout must not define the wire format.
- Unsupported versions and malformed messages must be
  rejected.
- Decoding must not read beyond message boundaries.
- Encoding and decoding must preserve logical event values.

### Ownership

Protocol message representations and temporary encoding
or decoding state.

## 5. maxwell-sequencer

### Responsibility

Validates event ordering and identifies sequence anomalies.

### Capabilities

- Expected sequence tracking.
- Duplicate detection.
- Gap detection.
- Missing sequence range representation.
- Sequence-related state transitions.

### Invariants

- Sequence advancement must follow the acceptance contract.
- Duplicates must not advance the expected cursor.
- Missing ranges must preserve correct boundaries.
- Sequence arithmetic must not silently wrap.

### Ownership

Sequence-validation state.

Within the integrated server, upstream sequence ownership
is coordinated by the recovery manager.

## 6. maxwell-book

### Responsibility

Reconstructs market state from validated events.

### Capabilities

- Instrument registration.
- Resting order storage.
- Dense price-level structures.
- Order addition and modification.
- Cancellation, deletion, and execution.
- Price-level aggregation.
- Book consistency validation.

### Invariants

- Order identifiers remain unique within their scope.
- Resting quantities remain valid.
- Prices remain within instrument constraints.
- Price-level aggregates reconcile with resting orders.
- Failed mutations preserve consistency.
- Logical order limits remain enforced.

### Ownership

Mutable instrument and order book state.

### Engineering Notes

Dense ladder capacity must accommodate the complete
instrument price band.

The current order registry uses hash-based storage with
logical capacity enforcement and fallible reservation.

A fixed-capacity registry and additional FIFO-priority
verification remain future improvements.

## 7. maxwell-transport

### Responsibility

Provides communication, framing, and buffering primitives.

### Capabilities

- Stream and datagram communication abstractions.
- Message framing.
- Bounded buffering.
- Transport-level error handling.

### Invariants

- Partial reads must not be treated as complete messages.
- Partial writes must not silently discard pending data.
- Frame lengths must be validated.
- Resource limits must remain explicit.

### Ownership

Transport state and communication buffers.

### Integration Status

Production server socket orchestration remains Phase III
work. Transport primitives alone do not constitute a
complete distributed market data service.

## 8. maxwell-recovery

### Responsibility

Coordinates restoration of synchronized market state.

### Capabilities

- Upstream sequence coordination.
- Missing-range identification.
- Bounded event buffering.
- Retransmission recovery.
- Snapshot recovery coordination.
- Recovery lifecycle management.

### Invariants

- Recovered events must preserve authoritative ordering.
- Duplicate events must not be applied twice.
- Failed application must not advance recovery progress.
- Synchronization must only be restored after satisfying
  the recovery contract.

### Ownership

Recovery state, upstream sequence cursor, and recovery
buffers.

### Integration Status

Correlated remote recovery requests and complete upstream
snapshot installation require further application
integration.

## 9. maxwell-publisher

### Responsibility

Distributes market data publications to subscribers.

### Capabilities

- Subscription registration.
- Instrument and event filtering.
- Bounded subscriber queues.
- Publication sequence tracking.
- Subscriber activation.
- Backpressure policies.
- Subscriber invalidation.

### Invariants

- Accepted publications preserve their defined ordering.
- Rejected atomic publications must not partially fan out.
- Invalidated subscribers require resynchronization.
- Failed sink delivery must retain pending data.
- Publication sequences remain independent of upstream
  and connection sequences.

### Ownership

Subscriber definitions, publication queues, and
publication sequence state.

## 10. maxwell-gateway

### Responsibility

Coordinates downstream sessions and subscriber lifecycle.

### Capabilities

- Session registration.
- Subscription coordination.
- Synchronization state management.
- Connection sequence tracking.
- Downstream delivery coordination.
- Session cleanup.

### Invariants

- Session identities must remain unambiguous.
- Incremental delivery must respect synchronization state.
- Connection sequences must remain independent of
  publication and upstream sequences.
- Invalidated sessions must not resume normal delivery
  without satisfying their recovery contract.

### Ownership

Downstream session state.

## 11. maxwell-client

### Responsibility

Provides consumer-side market data processing.

### Capabilities

- Downstream message consumption.
- Client-side sequence validation.
- Snapshot and incremental coordination.
- Local market state reconstruction.
- Synchronization tracking.

### Invariants

- Invalid or missing data must affect synchronization
  status appropriately.
- Initialization must establish a valid baseline.
- Subsequent increments must respect the baseline
  watermark.
- Unsynchronized client state must not be advertised
  as authoritative.

### Ownership

Consumer-side market state and synchronization state.

## 12. maxwell-metrics

### Responsibility

Provides operational instrumentation primitives.

### Capabilities

- Counters.
- Atomic counters.
- Gauges.
- Fixed-capacity latency histograms.
- Metric registration.
- Metric snapshots.
- Prometheus-compatible text representation.

### Invariants

- Counter overflow must follow documented semantics.
- Histogram bounds must remain consistent.
- Histogram aggregation requires compatible configurations.
- Instrumentation must not mutate financial market state.

### Ownership

Metric values and registry state.

Production monitoring endpoints and cloud exporters
remain separate integration concerns.

## 13. maxwell-server

### Responsibility

Integrates the foundational crates into a coherent
market data processing application.

### Capabilities

- Configuration and lifecycle management.
- Instrument registration.
- Empty-market initialization.
- Upstream event ingestion.
- Recovery coordination.
- Market mutation.
- Publication generation.
- Subscriber coordination.
- Pending publication handling.
- Operational status reporting.

### Invariants

- The server owns its integrated mutable processing state.
- The recovery manager owns the upstream sequence cursor.
- An accepted event must not be applied twice.
- Publication backpressure must not cause repeated
  market mutation.
- Unsynchronized state must not be advertised as valid.
- Snapshot activation must preserve watermark consistency.

### Ownership

Integrated market processing state and lifecycle.

### Current Boundaries

The initial application supports in-process integration.

Production socket orchestration, remote recovery
correlation, and complete upstream snapshot installation
remain future work.

## 14. maxwell-simulator

### Responsibility

Generates deterministic synthetic market workloads.

### Capabilities

- Independent authoritative synthetic market.
- Seeded random event generation.
- Configurable event distributions.
- Multiple instruments.
- Delivery fault injection.
- Bounded delivery queues.
- Retained event history.
- Snapshot generation.
- Virtual, unbounded, and real-time pacing.
- Operational workload statistics.

### Scenarios

| Scenario | Purpose |
|----------|---------|
| clean | Baseline ordered workload |
| smoke | Small correctness workload |
| high-throughput | Large unpaced workload |
| packet-loss | Missing event recovery |
| duplicates | Duplicate event handling |
| out-of-order | Reordering and sequencing |
| mixed-faults | Combined delivery faults |
| recovery-stress | Frequent recovery conditions |
| order-churn | Frequent insertion and removal |
| capacity-stress | Resource capacity testing |

### Invariants

- The authoritative market remains independent of the
  server's reconstructed market.
- Market and fault randomness remain independent.
- Identical seeds and configurations produce reproducible
  logical workloads.
- Delivery faults must not corrupt the authoritative market.
- Blocked delivery must retain the pending event.
- Generated and delivered events must be accounted for.

### Ownership

Synthetic market state, random generator state, retained
history, delivery queues, and simulation statistics.

## 15. maxwell-replay

### Responsibility

Provides historical market event persistence, validation,
and playback.

### Principal Components

| Component | Responsibility |
|-----------|----------------|
| config | Journal and replay configuration |
| error | Persistence and playback errors |
| format | Binary journal representation |
| record | Journal records and stream validation |
| writer | Buffered append-only recording |
| reader | Sequential validation and decoding |
| index | Sparse sequence and timestamp indexing |
| clock | Playback timing |
| filter | Record selection |
| replay | Playback orchestration |
| recorder | Recording lifecycle |
| stats | Recording and replay statistics |

### Capabilities

- Versioned binary journal headers.
- CRC32C record integrity checks.
- Authoritative and raw-capture journals.
- Configurable buffering and durability.
- Strict and incomplete-tail reading policies.
- Sparse indexing.
- Sequence and timestamp seeking.
- Instrument, event, sequence, and timestamp filtering.
- Virtual, unbounded, real-time, and scaled playback.
- Explicit replay sink backpressure.

### Invariants

- Original upstream event identity must be preserved.
- Authoritative records must remain contiguous.
- Recording timestamps must not regress.
- Raw capture preserves arrival order.
- Corruption must not be silently accepted.
- Backpressured records remain pending until accepted.
- Filtered or seeked replay is not automatically
  authoritative.
- Failed writer operations may poison the recording session.

### Ownership

Journal file handles, encoding buffers, reader state,
sparse indexes, playback clocks, and pending records.

### Current Boundaries

- One feed identity per journal.
- No reopening an existing journal for append.
- In-memory sparse indexing.
- No production journal rotation or remote storage.
- No transactional coupling between market mutation
  and persistence.
- No durable checkpoint or snapshot catalog.

## 16. maxwell-cli

### Responsibility

Provides the unified Maxwell operational command line.

The package produces an executable named maxwell.

### Commands

| Command | Responsibility |
|---------|----------------|
| help | Display command usage |
| version | Display application version |
| scenarios | List predefined workloads |
| simulate | Execute a local simulation |
| record | Record simulated market events |
| inspect | Inspect journal metadata |
| validate | Validate journal contents |
| replay | Replay historical events |
| benchmark | Measure simulator throughput |

### Principal Components

| Component | Responsibility |
|-----------|----------------|
| args | Strict command-line parsing |
| config | Command-specific configuration |
| error | Unified operational errors |
| output | Human-readable reporting |
| commands | Command dispatch and orchestration |

### Invariants

- Domain logic remains in the underlying libraries.
- Invalid arguments are rejected explicitly.
- Existing journal files must not be overwritten.
- Authoritative replay restrictions must be enforced.
- Benchmark results must identify their measurement scope.
- Operational failures must return nonzero exit codes.

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Operational failure |
| 2 | Invalid command or usage |

### Current Boundaries

The initial CLI is local and human-readable.

Remote administration, structured JSON reporting, and
distributed operational commands remain future extensions.

## 17. Component Ownership Matrix

| State | Primary Owner |
|-------|---------------|
| Upstream sequence cursor | Recovery manager |
| Resting market orders | Book |
| Publication sequence | Publisher |
| Subscriber queues | Publisher |
| Connection sequence | Gateway session |
| Consumer market state | Client |
| Integrated lifecycle | Server |
| Synthetic authoritative market | Simulator |
| Random generator state | Simulator |
| Journal writer state | Replay writer |
| Historical read position | Replay reader |
| Pending replay record | Replay engine |
| Command configuration | CLI |

Mutable state should not have multiple independent owners.

## 18. Change Impact Matrix

| Changed Component | Review Related Components |
|-------------------|---------------------------|
| Core | All domain consumers |
| Protocol | Transport, server, replay, Planck adapter |
| Sequencer | Recovery, server, client |
| Book | Server, recovery, reconstruction tests |
| Transport | Server, gateway, client |
| Recovery | Server, simulator, client |
| Publisher | Server, gateway, client |
| Gateway | Server, publisher, client |
| Client | Gateway, protocol |
| Metrics | Server, simulator, operational reporting |
| Server | Simulator integration, replay reconstruction |
| Simulator | Server integration, recording, benchmarks |
| Replay | CLI, reconstruction tests |
| CLI | Simulator and replay interfaces |

## 19. Adding or Modifying a Component

A component change should identify:

- Its responsibility.
- Its public interface.
- Its mutable state owner.
- Its resource limits.
- Its failure semantics.
- Its sequence requirements.
- Its synchronization requirements.
- Its integration dependencies.
- Its validation strategy.

Avoid introducing application orchestration into foundational
domain crates.

## 20. Validation

Generate the public API documentation:

    cargo doc --workspace --no-deps

Inspect workspace dependencies:

    cargo tree --workspace

Validate an individual package:

    cargo test -p maxwell-replay

Validate the complete workspace:

    cargo test --workspace
    cargo clippy --workspace --all-targets -- -D warnings

## 21. Related Documentation

- [Architecture](architecture.md)
- [Concurrency](concurrency.md)
- [Protocol](protocol.md)
- [Design Decisions](decisions.md)
- [Testing](testing.md)
