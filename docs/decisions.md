# Architectural Decision Records

## 1. Purpose

This document records significant architectural decisions
made during Maxwell's development.

Architectural Decision Records (ADRs) preserve the context,
rationale, alternatives, and consequences of technical
decisions.

An accepted decision represents an architectural direction.
It does not necessarily imply that every associated
application-level capability has been implemented.

## 2. Decision Index

| ADR | Decision | Status |
|-----|----------|--------|
| 001 | Use Rust for market data infrastructure | Accepted |
| 002 | Separate matching from market data processing | Accepted |
| 003 | Organize Maxwell as a Cargo workspace | Accepted |
| 004 | Use fixed-point financial representations | Accepted |
| 005 | Prioritize deterministic processing | Accepted |
| 006 | Adopt single-owner mutable state | Accepted |
| 007 | Bound resource consumption | Accepted |
| 008 | Use explicit backpressure policies | Accepted |
| 009 | Separate protocol and transport | Accepted |
| 010 | Separate sequencing and recovery | Accepted |
| 011 | Distinguish sequence domains | Accepted |
| 012 | Coordinate snapshot activation | Accepted |
| 013 | Prefer safe Rust | Accepted |
| 014 | Minimize external dependencies | Accepted |
| 015 | Keep instrumentation owner-local | Accepted |
| 016 | Use fixed-capacity latency histograms | Accepted |
| 017 | Separate libraries from applications | Accepted |
| 018 | Establish correctness before optimization | Accepted |
| 019 | Introduce parallelism incrementally | Accepted |
| 020 | Require deterministic simulation and replay | Accepted |

## ADR-001: Use Rust for Market Data Infrastructure

### Context

Market data processing requires predictable performance,
memory safety, and efficient systems-level programming.

Planck is implemented in C++23.

### Decision

Implement Maxwell in Rust.

### Alternatives Considered

- C++.
- C.
- Zig.
- Go.
- Java.

### Rationale

Rust provides:

- Native compilation.
- Explicit ownership.
- Memory safety without mandatory garbage collection.
- Strong domain type modeling.
- Efficient abstractions.
- Integrated package management and testing.

### Consequences

Cross-language interoperability with Planck requires an
explicit protocol contract.

Rust ownership and borrowing requirements influence
component boundaries and concurrency design.

## ADR-002: Separate Matching from Market Data Processing

### Context

Matching and market data distribution have different
performance characteristics and scaling requirements.

### Decision

Maintain Planck and Maxwell as independent systems.

Planck owns execution state. Maxwell owns derived market
data state.

### Alternatives Considered

- Embed market data distribution inside Planck.
- Share mutable order book memory between both systems.
- Combine execution and publication in one process.

### Rationale

Independent systems provide:

- Clear responsibility boundaries.
- Independent scaling.
- Independent deployment.
- Failure isolation.
- Reduced interference with order execution.

### Consequences

An explicit interoperability contract is required.

Maxwell must handle transport failures, sequence gaps,
and synchronization independently.

Process separation does not eliminate the need to manage
upstream resource consumption.

## ADR-003: Organize Maxwell as a Cargo Workspace

### Context

The platform contains multiple reusable components with
distinct responsibilities.

### Decision

Use a Cargo workspace containing focused library crates
and executable applications.

### Alternatives Considered

- One monolithic crate.
- Independent repositories for every component.

### Rationale

A workspace provides coordinated builds, shared dependency
management, independent component testing, and explicit
dependency boundaries.

### Consequences

Public crate interfaces must be maintained carefully.

Circular dependencies must be avoided.

Excessive crate fragmentation should also be avoided.

## ADR-004: Use Fixed-Point Financial Representations

### Context

Prices and quantities require consistent arithmetic and
comparison semantics.

### Decision

Represent financial values using validated integer-based
fixed-point domain types.

### Alternatives Considered

- IEEE 754 floating-point values.
- Arbitrary-precision decimal libraries.
- Untyped integer values throughout the application.

### Rationale

Fixed-point representations provide predictable comparison,
explicit precision, and reproducible arithmetic.

Domain types prevent accidental mixing of unrelated values.

### Consequences

Scale, range, conversion, and rounding rules must be
explicitly documented.

Overflow and underflow must be handled deliberately.

Fixed-point arithmetic is not inherently overflow-safe.

## ADR-005: Prioritize Deterministic Processing

### Context

Market data state must be reproducible for debugging,
recovery validation, and historical replay.

### Decision

Identical ordered logical inputs and equivalent initial
state should produce identical logical results.

### Alternatives Considered

- Allow processing order to depend on thread scheduling.
- Accept nondeterministic publication ordering.

### Rationale

Determinism simplifies:

- Regression testing.
- Historical replay.
- Root-cause analysis.
- Recovery verification.
- State reconciliation.

### Consequences

Iteration order, merge order, partitioning, and scheduling
policies require explicit consideration.

Wall-clock measurements and operational telemetry are not
necessarily deterministic.

## ADR-006: Adopt Single-Owner Mutable State

### Context

Shared mutable state introduces contention and makes
state transitions more difficult to reason about.

### Decision

Assign one logical owner to each mutable processing
component.

Prefer message passing and explicit ownership transfer.

### Alternatives Considered

- Shared order books protected by mutexes.
- Concurrent mutation through fine-grained locks.
- Atomic fields throughout processing structures.

### Rationale

Single ownership reduces synchronization requirements and
simplifies consistency guarantees.

### Consequences

Communication boundaries require explicit capacity and
ordering policies.

Single ownership does not imply that the entire application
must execute on one thread.

## ADR-007: Bound Resource Consumption

### Context

Unrestricted queues and buffers can turn temporary
overload into process-wide resource exhaustion.

### Decision

Use explicit capacity limits for growing runtime structures.

### Alternatives Considered

- Unbounded channels.
- Dynamically expanding recovery buffers.
- Unrestricted subscriber queues.

### Rationale

Bounded resources make overload behavior more predictable
and improve operational isolation.

### Consequences

Every capacity limit requires a documented exhaustion
policy.

Capacity selection becomes an operational configuration
responsibility.

Bounded memory is not equivalent to allocation-free
processing.

## ADR-008: Use Explicit Backpressure Policies

### Context

Downstream consumers may process data more slowly than
the publisher generates it.

### Decision

Handle overload using explicit policy outcomes.

The publisher supports:

- RejectPublication.
- RequireResync.

### Alternatives Considered

- Silently drop incremental updates.
- Block indefinitely.
- Grow subscriber queues without limits.

### Rationale

Explicit policies preserve correctness and expose overload
to the application.

### Consequences

RejectPublication requires application-managed retry.

RequireResync requires subscriber invalidation and a
coordinated synchronization procedure.

Neither policy eliminates the need for overload monitoring.

## ADR-009: Separate Protocol and Transport

### Context

Market data messages may be delivered through different
communication mechanisms.

### Decision

Separate binary message representation from transport
implementation.

### Alternatives Considered

- Embed message parsing inside socket handlers.
- Couple protocol messages directly to one transport.

### Rationale

Separation improves testability, replay support, and
transport portability.

### Consequences

Framing and message boundary responsibilities must be
explicit.

Transport delivery guarantees cannot be inferred from
protocol structure.

## ADR-010: Separate Sequencing and Recovery

### Context

Detecting a sequence gap is distinct from deciding how
to restore missing state.

### Decision

Keep sequence validation and recovery orchestration in
separate components.

### Alternatives Considered

- Implement all recovery logic inside the sequencer.
- Allow the order book to manage upstream recovery.

### Rationale

Sequencing remains focused on continuity.

Recovery manages buffering, retransmission, snapshots,
and synchronization policy.

### Consequences

The application must coordinate the two components.

Recovery completion must update sequencing state
consistently with restored market state.

## ADR-011: Distinguish Sequence Domains

### Context

Upstream feed sequences and downstream publication
sequences represent different ordering relationships.

### Decision

Treat feed, recovery, publication, and session sequence
domains independently.

### Alternatives Considered

- Use upstream sequence numbers for all downstream delivery.
- Treat publication sequence gaps as universal packet loss.

### Rationale

Filtering and recovery can produce legitimate differences
between sequence domains.

### Consequences

Protocol documentation must identify sequence scope.

Filtered subscribers cannot infer missing relevant updates
solely from gaps in global publication sequences.

## ADR-012: Coordinate Snapshot Activation

### Context

A downstream subscriber requires a consistent starting
state before processing incremental updates.

### Decision

Associate snapshot activation with an explicit publication
watermark.

### Alternatives Considered

- Begin incremental delivery immediately upon subscription.
- Generate snapshots independently of publication state.
- Maintain unrestricted publication history.

### Rationale

A coordinated boundary prevents missing or duplicate
incremental state transitions.

### Consequences

Snapshot generation and activation require application-level
coordination.

The initial publisher implementation does not maintain a
general historical replay log.

## ADR-013: Prefer Safe Rust

### Context

Unsafe code can introduce memory safety defects that are
difficult to detect through ordinary testing.

### Decision

Prefer safe Rust throughout the foundational crates.

### Alternatives Considered

- Introduce unsafe data structures preemptively.
- Use unchecked indexing in critical paths.
- Implement custom memory management immediately.

### Rationale

Safe Rust provides strong correctness guarantees while
retaining native performance.

### Consequences

Some optimizations may require additional design work.

Any proposed unsafe code must include documented invariants,
a measurable benefit, and a dedicated review.

## ADR-014: Minimize External Dependencies

### Context

External dependencies increase maintenance, security,
compatibility, and supply-chain obligations.

### Decision

Prefer standard library functionality when sufficient.

### Alternatives Considered

- Adopt a large asynchronous runtime immediately.
- Use external frameworks for every subsystem.
- Implement all functionality internally regardless of cost.

### Rationale

A focused dependency graph improves maintainability and
reduces unnecessary abstractions.

### Consequences

Dependencies remain acceptable when they provide a clear
benefit.

Avoiding dependencies must not justify implementing
security-sensitive algorithms incorrectly.

## ADR-015: Keep Instrumentation Owner-Local

### Context

Shared metrics can introduce contention into otherwise
independent processing threads.

### Decision

Prefer single-owner counters, gauges, and histograms.

Use atomic counters only when shared instrumentation is
genuinely required.

### Alternatives Considered

- Global mutex-protected metrics registry.
- Atomic operations for every metric update.
- Synchronous logging of every market event.

### Rationale

Owner-local metrics reduce synchronization overhead and
cache-line contention.

### Consequences

The application requires a safe telemetry collection
mechanism.

Metrics snapshots may represent different collection times
unless explicitly coordinated.

## ADR-016: Use Fixed-Capacity Latency Histograms

### Context

Latency measurements must remain bounded under sustained
high-throughput workloads.

### Decision

Use integer histogram buckets with fixed capacity.

Maintain exact accepted observation counts, sums, minimums,
and maximums.

Estimate percentiles using bucket boundaries.

### Alternatives Considered

- Retain every latency observation.
- Maintain dynamically growing distributions.
- Introduce an external histogram implementation immediately.

### Rationale

Fixed-capacity histograms provide predictable storage and
allocation-free recording.

### Consequences

Percentile estimates depend on bucket resolution.

Overflow observations may produce coarse tail estimates.

Histogram configurations must match before aggregation.

## ADR-017: Separate Libraries from Applications

### Context

Core processing functionality should be reusable across
live processing, simulation, and replay.

### Decision

Keep reusable processing logic in library crates.

Executable applications act as composition roots.

### Alternatives Considered

- Implement processing logic directly in the server.
- Duplicate processing logic across applications.

### Rationale

Separation improves testability and reduces behavioral
differences between live and simulated processing.

### Consequences

Application integration must explicitly coordinate component
state transitions and error handling.

## ADR-018: Establish Correctness Before Optimization

### Context

Premature optimization can introduce subtle market data
consistency defects.

### Decision

Establish a correct, measurable baseline before introducing
complex optimizations.

### Alternatives Considered

- Optimize every component during initial implementation.
- Assume low-level implementation techniques guarantee speed.

### Rationale

Profiling provides evidence about actual bottlenecks.

Correctness tests provide a reference for optimized behavior.

### Consequences

Initial implementations may not achieve final performance
objectives.

Optimizations require regression testing and measurement.

## ADR-019: Introduce Parallelism Incrementally

### Context

Additional processing threads may improve throughput but
can introduce queueing, contention, and ordering complexity.

### Decision

Begin with a simple processing topology.

Introduce additional ownership partitions only when
correctness requirements and profiling justify them.

### Alternatives Considered

- One thread per crate.
- One thread per instrument.
- General-purpose worker pools for every event.

### Rationale

Logical component boundaries do not necessarily correspond
to optimal thread boundaries.

### Consequences

The final server topology remains subject to benchmarking.

Parallel publication requires explicit ordering semantics.

## ADR-020: Require Deterministic Simulation and Replay

### Context

Live market conditions are difficult to reproduce.

### Decision

Provide dedicated simulation and replay applications using
shared processing components.

### Alternatives Considered

- Test exclusively against a live Planck instance.
- Generate uncontrolled random workloads.
- Maintain separate test-only processing implementations.

### Rationale

Deterministic workloads enable reproducible debugging,
failure injection, and performance comparisons.

### Consequences

Simulation seeds, event ordering, configuration, and
initial state must be recorded.

Replay cannot reproduce external timing behavior unless
that behavior is captured and modeled explicitly.

## 3. Adding a New Decision

Use the following template:

```text
ADR-NNN: Decision Title

Status:
Proposed | Accepted | Rejected | Superseded

Context:
What problem requires a decision?

Decision:
What approach is selected?

Alternatives:
What other approaches were considered?

Rationale:
Why was this approach selected?

Consequences:
What benefits, limitations, and obligations follow?

Validation:
How will the decision be verified?

Related:
Relevant ADRs, components, and documentation.
```

Do not rewrite historical decisions to conceal changes.

When a decision is replaced, mark it superseded and link
to the replacement ADR.

## 4. Decisions Requiring Future Review

The following topics remain open for application integration:

- Final server thread topology.
- Network event loop implementation.
- Upstream and downstream transport selection.
- Planck wire-format compatibility.
- Feed partitioning.
- Stream epoch and reconnect semantics.
- Session authentication.
- Runtime configuration format.
- Persistent market data recording.
- Production deployment topology.
- Quantitative latency and throughput requirements.
- Telemetry snapshot transfer mechanism.
