# Maxwell Crate Reference

## 1. Purpose

This document describes the responsibilities, ownership
boundaries, invariants, and extension points of Maxwell's
library crates.

It is intended for engineers maintaining individual
components or integrating them into applications.

Exact public signatures are defined by the source code and
generated Rust API documentation.

```bash
cargo doc --workspace --no-deps --open
```

Actual dependency relationships can be inspected using:

```bash
cargo tree --workspace
```

## 2. Component Directory

| Crate | Responsibility | Primary Maintenance Concern |
|-------|----------------|-----------------------------|
| maxwell-core | Domain primitives | Numerical correctness |
| maxwell-protocol | Binary messages | Wire compatibility |
| maxwell-sequencer | Sequence validation | Ordering correctness |
| maxwell-book | Market state | State invariants |
| maxwell-transport | Communication | Delivery semantics |
| maxwell-recovery | Synchronization | Recovery consistency |
| maxwell-publisher | Fan-out | Backpressure and ordering |
| maxwell-gateway | Sessions | Lifecycle coordination |
| maxwell-client | Consumer state | Synchronization |
| maxwell-metrics | Telemetry | Instrumentation overhead |

## 3. maxwell-core

Location:

```text
crates/maxwell-core/
```

### Responsibility

Define reusable domain types and foundational primitives.

### Domain Concepts

- Instrument identifiers.
- Price.
- Quantity.
- Sequence identifiers.
- Timestamps.
- Shared validation and errors.

### Invariants

- Financial values retain their defined scale.
- Invalid domain values are rejected.
- Arithmetic does not silently overflow.
- Domain types are not accidentally interchangeable.
- Wire conversion preserves documented representations.

### Ownership

Core values should generally be lightweight value types.

They should not own application-level processing state.

### Performance Considerations

Avoid unnecessary allocation, dynamic dispatch, and
expensive conversions in foundational types.

### Extension Points

- Additional domain identifiers.
- Checked financial arithmetic.
- Explicit unit conversions.
- Additional validation helpers.

### Maintenance Risks

Changes to price, quantity, timestamp, or sequence
representations may affect every dependent component.

Such changes require protocol and interoperability review.

### Validation

```bash
cargo test -p maxwell-core
```

## 4. maxwell-protocol

Location:

```text
crates/maxwell-protocol/
```

### Responsibility

Represent, encode, decode, and validate binary market
data messages.

### Inputs

- Valid domain values.
- Binary message buffers.
- Protocol metadata.

### Outputs

- Encoded message bytes.
- Decoded market data messages.
- Explicit protocol errors.

### Invariants

- Reads remain within buffer boundaries.
- Encoded lengths match the documented format.
- Invalid message types are rejected.
- Numeric byte ordering is explicit.
- Financial values retain their wire representation.
- Unsupported messages do not silently become valid events.

### Ownership

Protocol components should not own order book state or
downstream session state.

### Performance Considerations

Prefer caller-provided buffers and bounded decoding.

Avoid unnecessary intermediate representations.

### Extension Points

- New message types.
- Protocol version support.
- Additional encoding helpers.
- Cross-language test vectors.

### Maintenance Risks

Changing message identifiers, field widths, byte ordering,
or framing can break Planck interoperability.

Any wire-format change requires compatibility review.

### Validation

```bash
cargo test -p maxwell-protocol
```

## 5. maxwell-sequencer

Location:

```text
crates/maxwell-sequencer/
```

### Responsibility

Track sequence continuity within a defined sequencing
scope.

### Inputs

- Incoming event sequence.
- Current expected sequence.
- Explicit synchronization or reset operations.

### Outputs

- Continuity result.
- Duplicate indication.
- Gap indication.
- Invalid sequence indication.
- Updated sequencing state.

### Invariants

- Accepted events advance sequence state according to
  the defined contract.
- Duplicate events do not advance state incorrectly.
- Missing sequences are identified explicitly.
- Sequence exhaustion does not silently wrap.
- Reset behavior is explicit.

### Ownership

Each mutable sequencing state belongs to one processing
owner.

### Extension Points

- Additional feed scopes.
- Explicit stream epochs.
- Sequence diagnostics.
- Additional sequencing policies.

### Maintenance Risks

Changing the interpretation of expected sequence values
can introduce off-by-one errors throughout recovery.

Review whether a watermark represents the last applied
sequence or the next expected sequence.

### Validation

```bash
cargo test -p maxwell-sequencer
```

## 6. maxwell-book

Location:

```text
crates/maxwell-book/
```

### Responsibility

Maintain deterministic market data order book state.

### State

- Instrument-specific market state.
- Bid and ask ladders.
- Price levels.
- Quantities.
- Relevant book statistics.

### Inputs

Validated and appropriately ordered market events.

### Outputs

- Updated market state.
- Best bid and offer.
- Price-level information.
- Snapshots.
- Explicit book errors.

### Invariants

- Price levels maintain their required ordering.
- Quantity arithmetic remains valid.
- Best-price calculations reflect current state.
- Invalid mutations cannot silently corrupt the book.
- Duplicate events are prevented by sequencing and
  application coordination.
- Snapshot state corresponds to a defined boundary.

### Ownership

Each mutable book belongs to one processing owner.

Read-only snapshots may be transferred to other owners
through safe, explicit mechanisms.

### Performance Considerations

Important operations include:

- Price-level lookup.
- Price-level insertion.
- Price-level removal.
- Best-price lookup.
- Snapshot generation.
- Instrument routing.

Data structure changes should be evaluated against realistic
book depth and update distributions.

### Extension Points

- Additional market event handling.
- Alternative price-ladder representations.
- Additional snapshot formats.
- Book reconciliation tools.
- Instrument-specific configuration.

### Maintenance Risks

Do not expose unrestricted internal mutation merely to
simplify integration.

Internal methods such as price-level setters should remain
private or crate-private unless a public contract is
genuinely required.

### Validation

```bash
cargo test -p maxwell-book
```

## 7. maxwell-transport

Location:

```text
crates/maxwell-transport/
```

### Responsibility

Provide transport abstractions, framing, buffering, and
communication error handling.

### Inputs

- Incoming transport bytes or frames.
- Outgoing encoded messages.
- Transport configuration.

### Outputs

- Received messages or frames.
- Delivery acceptance results.
- Backpressure indications.
- Transport errors.

### Invariants

- Buffer capacity is respected.
- Partial reads and writes are handled according to
  transport semantics.
- Frame boundaries are validated.
- Retry behavior does not duplicate accepted bytes.
- Transport failure is distinguishable from successful
  delivery.

### Ownership

Mutable transport state belongs to its designated I/O or
processing owner.

### Extension Points

- TCP adapters.
- UDP adapters.
- Multicast adapters.
- In-memory transports.
- Shared-memory transports.
- Recording and replay adapters.

These are extension possibilities, not a claim that every
adapter currently exists.

### Maintenance Risks

A successful local send operation does not establish
remote application receipt.

Nonblocking errors must not be confused with fatal
connection failures.

### Validation

```bash
cargo test -p maxwell-transport
```

## 8. maxwell-recovery

Location:

```text
crates/maxwell-recovery/
```

### Responsibility

Restore synchronization following missing or inconsistent
upstream market data.

### State

- Recovery lifecycle state.
- Affected feed scope.
- Missing sequence range.
- Buffered events.
- Recovery watermark.
- Replay progress.

### Inputs

- Sequencing results.
- Subsequent upstream events.
- Retransmission responses.
- Snapshot responses.
- Explicit recovery control operations.

### Outputs

- Recovery requests.
- Eligible ordered events.
- Synchronization transitions.
- Recovery failures.

### Invariants

- Recovery buffers remain bounded.
- Recovered events are validated.
- Snapshot watermarks are validated.
- Events represented by a snapshot are not replayed twice.
- Failed recovery does not advertise synchronized state.
- Replay progress reflects successfully applied events.

### Ownership

Recovery state should be coordinated with the corresponding
sequencer and order book owner.

### Extension Points

- Recovery timeout policies.
- Retry limits.
- Recovery source selection.
- Feed redundancy.
- Additional recovery diagnostics.

### Maintenance Risks

Recovery callbacks and state transitions must preserve
transactional consistency.

Advancing a replay cursor before successful state mutation
can permanently lose an event.

### Validation

```bash
cargo test -p maxwell-recovery
```

## 9. maxwell-publisher

Location:

```text
crates/maxwell-publisher/
```

### Responsibility

Filter and distribute accepted publications to bounded
subscriber queues.

### Principal Public Types

| Type | Purpose |
|------|---------|
| Publisher | Owns publication and subscriber state |
| PublisherConfig | Capacity and backpressure configuration |
| BackpressurePolicy | Defines overload behavior |
| Publication | Generic market data publication |
| EventKind | Publication category |
| SubscriptionFilter | Instrument and event selection |
| SubscriberId | Subscriber identity |
| SubscriberState | Synchronization lifecycle |
| Subscription | Subscriber state and pending deliveries |
| BoundedQueue | Fixed logical-capacity FIFO queue |
| PublicationSink | Nonblocking delivery abstraction |
| PublishReport | Publication outcome |
| DrainReport | Delivery progress |
| PublisherStats | Operational counters |

### Principal Operations

- Construct a publisher.
- Register and remove subscribers.
- Activate subscribers at a valid watermark.
- Publish events.
- Inspect pending publications.
- Remove or drain accepted deliveries.
- Invalidate subscribers.
- Inspect operational statistics.

### State

Subscribers are maintained in deterministic identifier order.

Each subscriber owns a bounded publication queue and a
subscription filter.

The publisher maintains a global publication sequence.

### Subscriber States

- PendingSnapshot.
- Active.
- ResyncRequired.

### Backpressure

RejectPublication rejects an event before partial fan-out
when a matching active subscriber lacks capacity.

RequireResync invalidates an overloaded subscriber and
allows healthy subscribers to continue.

### Invariants

- Publication sequence advances only for accepted
  publications.
- Matching active subscribers receive publications in
  relative publication order.
- Pending or invalidated subscribers do not receive
  ordinary incremental publications.
- Failed sink acceptance retains the pending publication.
- Subscriber queues remain bounded.
- Activation requires a valid current watermark.

### Complexity

For S subscribers and up to I selected instruments per
subscription, publication filtering is approximately
O(S log I) for the initial sorted-vector implementation.

Queue insertion and removal are constant-time operations
under normal preallocated usage.

### Maintenance Risks

Publication sequence gaps caused by filtering are valid.

The publisher is not a general-purpose historical replay
store.

Snapshot coordination and rejected-publication retries
belong to the application.

### Validation

```bash
cargo test -p maxwell-publisher
```

## 10. maxwell-gateway

Location:

```text
crates/maxwell-gateway/
```

### Responsibility

Coordinate downstream session and subscription lifecycles.

### State

- Session identifiers.
- Session lifecycle state.
- Subscription relationships.
- Pending delivery state.
- Synchronization status.

### Inputs

- Session control operations.
- Subscription requests.
- Snapshots.
- Publisher deliveries.
- Transport outcomes.

### Outputs

- Session responses.
- Downstream market data.
- Invalidation notifications.
- Session lifecycle transitions.
- Gateway errors.

### Invariants

- Session capacity remains bounded.
- Subscription ownership is unambiguous.
- Snapshot and incremental delivery ordering is preserved.
- Invalidated sessions do not silently resume.
- Disconnection releases associated resources according
  to the documented lifecycle.

### Ownership

Session state must have one logical owner.

If the publisher and gateway have different owners, their
interaction requires a bounded communication contract.

### Extension Points

- Network session adapters.
- Session authentication.
- Subscription authorization.
- Heartbeat scheduling.
- Session-local delivery sequencing.

### Maintenance Risks

Partial transport writes require explicit retry state.

Session invalidation must not depend on another market
event arriving.

### Validation

```bash
cargo test -p maxwell-gateway
```

## 11. maxwell-client

Location:

```text
crates/maxwell-client/
```

### Responsibility

Provide reusable client-side market data consumption and
synchronization logic.

### State

- Connection or session state.
- Subscription state.
- Synchronization status.
- Local market data state.
- Relevant sequence tracking.

### Inputs

- Session responses.
- Snapshots.
- Incremental market data.
- Invalidation notifications.
- Transport failures.

### Outputs

- Updated local market state.
- Client lifecycle transitions.
- Resynchronization requests.
- Explicit client errors.

### Invariants

- Incremental state is not considered synchronized before
  initialization completes.
- Invalidated state is not silently treated as current.
- Sequence domains are interpreted correctly.
- Snapshot application establishes a valid state boundary.
- Reconnect behavior follows the session contract.

### Extension Points

- Trading system adapters.
- Analytics adapters.
- Asynchronous client wrappers.
- Additional subscription policies.
- Client-side diagnostics.

### Maintenance Risks

Client logic must not interpret filtered global publication
sequence gaps as definitive message loss.

The exact downstream delivery continuity mechanism must be
defined by the protocol and gateway contract.

### Validation

```bash
cargo test -p maxwell-client
```

## 12. maxwell-metrics

Location:

```text
crates/maxwell-metrics/
```

### Responsibility

Provide bounded, low-overhead telemetry primitives.

### Principal Public Types

| Type | Purpose |
|------|---------|
| Counter | Saturating single-owner counter |
| Gauge | Signed saturating gauge |
| AtomicCounter | Cache-line-aligned atomic counter |
| Histogram | Fixed-capacity integer histogram |
| HistogramSnapshot | Owned histogram observations |
| MetricsRegistry | Bounded metric registry |
| MetricId | Registry-local metric identifier |
| MetricSnapshot | Owned metric representation |
| MetricsError | Instrumentation errors |

### Export

The write_prometheus function writes metric snapshots
using Prometheus-compatible text exposition.

The metrics crate does not itself provide an HTTP server.

### Histogram Semantics

- Bucket bounds are strictly increasing.
- Observations above the final bound enter an overflow
  bucket.
- Count, sum, minimum, and maximum are exact for accepted
  observations.
- Intermediate percentiles are bucket-based estimates.
- Histograms require matching bounds for aggregation.
- Count exhaustion rejects additional observations.

### Atomic Semantics

AtomicCounter uses relaxed atomic ordering for telemetry.

It is not an application synchronization primitive.

Saturating addition uses a compare-exchange retry loop.

### Registry Semantics

Metric names are validated at registration.

Metrics receive stable numeric identifiers local to their
registry.

Recording uses indexed access.

Registration capacity is explicitly bounded.

### Maintenance Risks

Do not introduce a globally shared metrics registry into
the critical processing path without measurement.

Do not reset cumulative counters during ordinary
Prometheus collection.

Do not average percentiles across processing owners.

Aggregate compatible histograms before calculating
combined percentiles.

### Validation

```bash
cargo test -p maxwell-metrics
```

## 13. Component Ownership Matrix

| State | Logical Owner |
|-------|---------------|
| Receive buffer | Transport owner |
| Decoding scratch space | Decoder or processing owner |
| Expected feed sequence | Sequencer owner |
| Recovery buffer | Recovery owner |
| Order book | Book owner |
| Publication sequence | Publisher owner |
| Subscriber queues | Publisher owner |
| Session state | Gateway owner |
| Consumer book | Client owner |
| Local telemetry | Instrumented owner |

A logical owner may manage multiple components within
one thread.

## 14. Change Impact Matrix

| Changed Component | Additional Review |
|-------------------|-------------------|
| Core | All dependent crates |
| Protocol | Transport, recovery, client, Planck |
| Sequencer | Recovery, book, replay |
| Book | Recovery, publisher, client |
| Transport | Server, gateway, recovery, client |
| Recovery | Sequencer, book, publisher |
| Publisher | Gateway, client, server |
| Gateway | Publisher, transport, client |
| Client | Gateway, protocol, CLI |
| Metrics | Instrumented applications |

This matrix describes logical integration impact rather
than exact Cargo dependencies.

## 15. Adding a Component

Before introducing a new crate, determine whether the
functionality belongs in an existing component.

A new crate should have:

- A clearly defined responsibility.
- A documented public interface.
- Explicit dependency boundaries.
- Explicit error semantics.
- Documented state ownership.
- Capacity and allocation policies.
- Unit and integration tests.
- Relevant performance considerations.

Avoid creating new crates solely to separate small groups
of closely related functions.

## 16. Component Review Checklist

- [ ] Public interfaces are narrowly scoped.
- [ ] Domain values are validated.
- [ ] Errors are explicit.
- [ ] State transitions are documented.
- [ ] Resource consumption is bounded.
- [ ] Ownership assumptions are preserved.
- [ ] Ordering guarantees are preserved.
- [ ] Failure behavior is tested.
- [ ] Dependencies are justified.
- [ ] Relevant integration tests pass.
- [ ] Documentation reflects actual behavior.
