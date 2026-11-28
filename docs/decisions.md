# Maxwell Architectural Decisions

## 1. Purpose

This document records architectural decisions, their
rationale, consequences, and review conditions.

Decisions should preserve the reasoning behind the
implementation rather than merely describe its current
structure.

## 2. Decision Index

| ADR | Decision |
|-----|----------|
| 001 | Use Rust for market data infrastructure |
| 002 | Separate matching from market data processing |
| 003 | Organize Maxwell as a Cargo workspace |
| 004 | Use fixed-point financial representations |
| 005 | Prioritize deterministic processing |
| 006 | Adopt single-owner mutable state |
| 007 | Bound resource consumption |
| 008 | Use explicit backpressure policies |
| 009 | Separate protocol and transport |
| 010 | Separate sequencing and recovery |
| 011 | Distinguish sequence domains |
| 012 | Coordinate snapshot activation |
| 013 | Prefer safe Rust |
| 014 | Minimize external dependencies |
| 015 | Keep instrumentation owner-local |
| 016 | Use fixed-capacity latency histograms |
| 017 | Separate libraries from applications |
| 018 | Establish correctness before optimization |
| 019 | Introduce parallelism incrementally |
| 020 | Require deterministic simulation and replay |
| 021 | Separate market mutation from publication retry |
| 022 | Maintain an independent simulator market |
| 023 | Separate market and fault randomness |
| 024 | Use versioned append-only binary journals |
| 025 | Distinguish authoritative and raw journals |
| 026 | Separate flushing from durability |
| 027 | Retain pending replay records |
| 028 | Use bounded sparse journal indexes |
| 029 | Provide a unified operational CLI |
| 030 | Define explicit benchmark boundaries |
| 031 | Deploy an incremental distributed baseline |

## ADR-001: Use Rust for Market Data Infrastructure

Status: Accepted

### Decision

Implement Maxwell in Rust while retaining C++23 for Planck.

### Rationale

Rust provides explicit ownership, memory safety, strong
domain modeling, and efficient systems programming.

### Consequences

Cross-language interoperability must use explicit protocol
contracts rather than shared compiler-specific layouts.

## ADR-002: Separate Matching from Market Data Processing

Status: Accepted

### Decision

Maintain independent matching and market data systems.

### Rationale

Matching and market data distribution have different
state ownership, throughput, and operational requirements.

### Consequences

Planck owns authoritative exchange execution state.

Maxwell reconstructs and distributes market data without
performing order matching.

## ADR-003: Organize Maxwell as a Cargo Workspace

Status: Accepted

### Decision

Separate foundational libraries from application packages
within a shared Cargo workspace.

### Rationale

Independent components improve testing, reuse, dependency
management, and engineering handover.

### Consequences

Public interfaces and dependency direction require
deliberate review.

## ADR-004: Use Fixed-Point Financial Representations

Status: Accepted

### Decision

Represent authoritative prices and quantities using
fixed-point domain types.

### Rationale

Financial correctness requires explicit precision and
predictable arithmetic.

### Consequences

Overflow, scaling, tick alignment, and instrument-specific
constraints must be validated.

## ADR-005: Prioritize Deterministic Processing

Status: Accepted

### Decision

Identical logical inputs and initial state should produce
equivalent logical results.

### Rationale

Determinism improves recovery, regression testing, and
historical reconstruction.

### Consequences

Randomness, time, ordering, and state ownership must be
explicit.

## ADR-006: Adopt Single-Owner Mutable State

Status: Accepted

### Decision

Assign each mutable market state object an explicit owner.

### Rationale

Single ownership simplifies ordering and reduces
synchronization requirements.

### Consequences

Cross-thread interaction should use explicit communication
or ownership transfer.

## ADR-007: Bound Resource Consumption

Status: Accepted

### Decision

Use explicit logical limits for market state, queues,
recovery buffers, and historical processing resources.

### Rationale

Unrestricted resource growth produces unpredictable
latency and failure behavior.

### Consequences

Capacity exhaustion requires explicit handling.

Logical bounds do not establish allocation-free execution.

## ADR-008: Use Explicit Backpressure Policies

Status: Accepted

### Decision

Represent capacity exhaustion through defined
backpressure outcomes.

### Rationale

Silent dropping or unrestricted buffering can corrupt
market state or exhaust resources.

### Consequences

Callers must distinguish accepted, rejected, pending,
and invalidated states.

## ADR-009: Separate Protocol and Transport

Status: Accepted

### Decision

Keep logical binary message representation independent
of socket and transport implementation.

### Rationale

Protocol messages may be carried over different
communication mechanisms.

### Consequences

Transport adapters must establish message boundaries and
handle partial communication.

## ADR-010: Separate Sequencing and Recovery

Status: Accepted

### Decision

Separate sequence validation from recovery orchestration.

### Rationale

Detecting a missing event and restoring synchronization
are distinct responsibilities.

### Consequences

The recovery manager coordinates upstream sequence
ownership within the integrated server.

## ADR-011: Distinguish Sequence Domains

Status: Accepted

### Decision

Maintain independent upstream, publication, and
connection sequence domains.

### Rationale

Each sequence identifies ordering within a different
logical boundary.

### Consequences

Sequence values must not be substituted across domains.

Filtered publication gaps do not automatically indicate
upstream data loss.

## ADR-012: Coordinate Snapshot Activation

Status: Accepted

### Decision

Activate incremental delivery only after establishing
a valid snapshot or initialization watermark.

### Rationale

Inconsistent initialization boundaries can produce
missing or duplicate market mutations.

### Consequences

Invalidated subscribers require explicit
resynchronization.

## ADR-013: Prefer Safe Rust

Status: Accepted

### Decision

Keep Maxwell's own implementation in safe Rust.

### Rationale

Memory safety and maintainability are foundational
requirements.

### Consequences

Any future proposal involving unsafe code requires
explicit architectural review and supporting evidence.

## ADR-014: Minimize External Dependencies

Status: Accepted

### Decision

Introduce third-party dependencies only when they
provide a clear engineering benefit.

### Rationale

A smaller dependency surface improves auditability,
portability, and build predictability.

### Consequences

Additional dependencies must justify their complexity
and maintenance cost.

This decision does not prohibit useful future libraries.

## ADR-015: Keep Instrumentation Owner-Local

Status: Accepted

### Decision

Prefer owner-local instrumentation on critical
processing paths.

### Rationale

Shared metrics can introduce unnecessary synchronization
and cache-coherence traffic.

### Consequences

Cross-thread collection and aggregation must have explicit
consistency semantics.

## ADR-016: Use Fixed-Capacity Latency Histograms

Status: Accepted

### Decision

Represent latency distributions using bounded histogram
storage.

### Rationale

Unrestricted storage of individual latency samples is
unsuitable for sustained high-throughput processing.

### Consequences

Percentile accuracy depends on histogram resolution.

Aggregation requires compatible histogram configurations.

## ADR-017: Separate Libraries from Applications

Status: Accepted

### Decision

Keep domain and component logic in reusable libraries.

Applications coordinate those components.

### Rationale

The same implementation should support integration tests,
operational commands, and future deployment environments.

### Consequences

CLI and server orchestration must not duplicate core
financial logic.

## ADR-018: Establish Correctness Before Optimization

Status: Accepted

### Decision

Require correctness validation before introducing
performance-oriented changes.

### Rationale

A fast but inconsistent market data system is not useful.

### Consequences

Optimization requires reproducible benchmarks and
regression testing.

## ADR-019: Introduce Parallelism Incrementally

Status: Accepted

### Decision

Begin with deterministic single-owner processing and
introduce parallelism where justified.

### Rationale

Premature concurrency increases coordination complexity
and makes failure diagnosis more difficult.

### Consequences

Future partitioning requires explicit sequencing,
snapshot, publication, and shutdown contracts.

## ADR-020: Require Deterministic Simulation and Replay

Status: Accepted

### Decision

Support reproducible synthetic workloads and historical
event playback.

### Rationale

Deterministic inputs enable repeatable correctness
validation and recovery testing.

### Consequences

Random seeds, initial state, event identity, and
sequence ordering must be preserved.

## ADR-021: Separate Market Mutation from Publication Retry

Status: Accepted

### Context

A server may successfully mutate its market state before
downstream publication encounters backpressure.

Retrying the original event could apply the mutation twice.

### Decision

Represent consumed input with pending publication as a
distinct processing outcome.

Retain pending publication state and retry publication
rather than repeating the market mutation.

### Consequences

Ingestion adapters must distinguish input backpressure
from publication backpressure.

Tests must verify exactly-once logical mutation across
publication retries.

## ADR-022: Maintain an Independent Simulator Market

Status: Accepted

### Context

Using the server's order book as the simulator's expected
state would allow shared implementation defects to remain
undetected.

### Decision

Maintain an independent authoritative synthetic market.

### Rationale

Independent expected state provides stronger correctness
validation.

### Consequences

The simulator requires its own market representation and
consistency checks.

Canonical comparison must account for equivalent logical
state rather than identical internal storage.

## ADR-023: Separate Market and Fault Randomness

Status: Accepted

### Decision

Maintain independent deterministic randomness for market
generation and delivery fault injection.

### Rationale

Changing packet-loss or reordering behavior should not
unintentionally redefine the underlying market workload.

### Consequences

Market and delivery randomness require independent
initialization and state.

Tests should verify reproducibility across equivalent
configurations.

## ADR-024: Use Versioned Append-Only Binary Journals

Status: Accepted

### Decision

Persist events in a versioned binary journal containing
a fixed-size header and checksummed variable-length records.

Reuse native Maxwell protocol encoding within each record.

### Rationale

Binary persistence preserves exact event values, reduces
serialization duplication, and supports deterministic
historical playback.

### Consequences

Journal compatibility must be reviewed independently of
wire protocol compatibility.

The initial writer creates a new file and appends records
during that recording session.

Reopening an existing journal for append is not supported.

## ADR-025: Distinguish Authoritative and Raw Journals

Status: Accepted

### Context

Historical data may represent either a complete ordered
stream or the actual arrival order of a faulty transport.

### Decision

Support separate Authoritative and RawCapture modes.

### Rationale

A faulty arrival stream is useful for recovery testing
but must not be confused with authoritative history.

### Consequences

Authoritative records require contiguous upstream
sequences.

Raw capture permits gaps, duplicates, and reordering.

Both modes preserve original event identity.

Filtered or seeked playback requires additional baseline
consideration before authoritative reconstruction.

## ADR-026: Separate Flushing from Durability

Status: Accepted

### Decision

Expose buffered writing, flushing, and file-data
synchronization as distinct persistence operations.

### Rationale

Different durability policies have different latency and
storage performance characteristics.

### Consequences

A successful buffered write does not imply persistence
across power loss.

File-data synchronization does not establish directory
entry durability.

Production transactional persistence remains future work.

## ADR-027: Retain Pending Replay Records

Status: Accepted

### Decision

Retain the current replay record until the sink accepts it.

### Rationale

Advancing the reader's logical delivery progress before
acceptance can silently lose historical events.

### Consequences

Backpressure attempts must not increment accepted-record
statistics.

The caller must explicitly handle exhausted backpressure
budgets.

## ADR-028: Use Bounded Sparse Journal Indexes

Status: Accepted

### Decision

Use configurable sparse in-memory indexes rather than
retaining every journal record in memory.

### Rationale

Large historical journals require bounded memory
consumption while supporting useful seeking operations.

### Consequences

Index construction may require a complete journal scan.

Seeking may require sequential scanning from an indexed
position.

Persistent indexes and segmented journal catalogs remain
future improvements.

## ADR-029: Provide a Unified Operational CLI

Status: Accepted

### Decision

Expose local operational commands through the maxwell
executable provided by maxwell-cli.

Use existing libraries for domain processing.

### Rationale

A unified CLI simplifies development, testing, diagnostics,
and future deployment automation.

### Consequences

The initial CLI provides human-readable reporting and
strict argument parsing without introducing a third-party
argument parser.

Remote administration and structured output remain
future extensions.

## ADR-030: Define Explicit Benchmark Boundaries

Status: Accepted

### Decision

Measure simulator throughput separately from initialization,
post-run validation, networking, and journal persistence.

### Rationale

A benchmark is meaningful only when its measurement
boundary is understood.

### Consequences

Simulator throughput must not be represented as matching
engine throughput or end-to-end network latency.

Separate benchmarks are required for other components.

## ADR-031: Deploy an Incremental Distributed Baseline

Status: Proposed

### Context

Maxwell and Planck can be developed independently, but
their eventual integration requires real deployment and
operational validation.

Waiting for every production feature before deploying
would delay distributed systems feedback.

### Decision

Establish an initial distributed Maxwell environment
using the simulator as the market producer.

Introduce containerization and AWS infrastructure before
completing live Planck interoperability.

### Proposed Technology

- Docker.
- Terraform.
- AWS EC2.
- AWS VPC.
- Amazon S3.
- CloudWatch.
- GitHub Actions.

### Rationale

An incremental deployment exposes networking,
configuration, monitoring, and operational requirements
early.

### Consequences

The initial deployment must be explicitly described as
a development environment.

Production recovery, security, durability, and availability
guarantees must not be inferred from successful deployment.

## 3. Adding a New Decision

Each new ADR should include:

    ADR identifier:
    Title:
    Status:
    Context:
    Decision:
    Alternatives considered:
    Rationale:
    Consequences:
    Review conditions:

Recommended statuses:

- Proposed.
- Accepted.
- Superseded.
- Rejected.

Preserve superseded decisions to retain architectural
history.

## 4. Decisions Requiring Future Review

- Fixed-capacity order registry.
- Allocation behavior on the critical path.
- Instrument partitioning.
- Cross-partition publication ordering.
- Complete remote recovery protocol.
- Upstream snapshot installation.
- Journal rotation and retention.
- Persistent sparse indexes.
- Transactional persistence boundaries.
- Asynchronous recording.
- Structured CLI output.
- Cloud deployment topology.
- Clock synchronization.
- Production authentication and authorization.
- Performance regression thresholds.

## 5. Related Documentation

- [Architecture](architecture.md)
- [Concurrency](concurrency.md)
- [Protocol](protocol.md)
- [Performance](performance.md)
- [Roadmap](roadmap.md)
