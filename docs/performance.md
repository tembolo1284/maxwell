# Maxwell Performance Engineering

## 1. Purpose

This document defines Maxwell's performance engineering
methodology, measurement requirements, optimization
principles, and benchmark reporting standards.

Maxwell is designed for latency-sensitive electronic
trading infrastructure.

No production throughput or latency guarantees have
been established.

Performance claims must be supported by reproducible
measurements.

## 2. Performance Objectives

The principal objectives are:

- Low processing latency.
- Predictable tail latency.
- High sustained event throughput.
- Efficient subscriber fan-out.
- Bounded memory consumption.
- Minimal critical-path allocation.
- Minimal synchronization overhead.
- Predictable overload behavior.
- Efficient recovery.

Correctness remains the primary requirement.

A faster implementation that produces incorrect market
state is not an acceptable optimization.

## 3. Performance Terminology

| Term | Definition |
|------|------------|
| Throughput | Completed work per unit of time |
| Latency | Elapsed time for a defined operation |
| Service time | Time spent actively processing an operation |
| Queueing time | Time spent waiting before processing |
| Tail latency | High-percentile latency behavior |
| Jitter | Variation in observed latency |
| Fan-out | Distribution of one publication to multiple consumers |
| Saturation | Workload level at which a resource becomes limiting |

Measurements must define the operation being counted.

An upstream message, logical event, publication, and
subscriber delivery are not necessarily equivalent.

## 4. Measurement Boundaries

```mermaid
flowchart LR
    A["Planck Event"]
    B["Maxwell Receive"]
    C["Decode"]
    D["Sequence and Recover"]
    E["Book Mutation"]
    F["Publication"]
    G["Transport Delivery"]
    H["Client Receive"]

    A --> B --> C --> D --> E --> F --> G --> H
```

Recommended measurement boundaries:

| Measurement | Start | End |
|-------------|-------|-----|
| Decode latency | Decoder entry | Validated message |
| Sequencing latency | Sequencer entry | Sequence result |
| Book latency | Book mutation entry | Mutation completion |
| Publication latency | Publisher entry | Publication acceptance |
| Processing latency | Validated event entry | Publication acceptance |
| Queueing latency | Queue insertion | Queue removal |
| Server latency | Local message receipt | Defined send boundary |
| End-to-end latency | Planck event boundary | Client receipt |
| Recovery latency | Gap detection | Synchronization restored |

Every benchmark must identify its exact measurement
boundaries.

Local transport acceptance is not equivalent to remote
client receipt.

## 5. Throughput Metrics

Track throughput independently at each stage.

```text
ingestion_rate = received_messages / elapsed_seconds

processing_rate = processed_events / elapsed_seconds

publication_rate = accepted_publications / elapsed_seconds

delivery_rate = subscriber_deliveries / elapsed_seconds
```

For example, a publisher accepting 100,000 publications
per second and delivering each to ten subscribers may
produce 1,000,000 subscriber deliveries per second.

This is an illustrative calculation, not a measured
Maxwell result.

Report rejected events, discarded deliveries, recovery
activity, and queue growth alongside throughput.

High throughput is not meaningful if work is silently
discarded or queues grow without bound.

## 6. Latency Distributions

Report at least:

- Minimum.
- Mean.
- Median.
- P90.
- P95.
- P99.
- P99.9.
- P99.99 when sample size is sufficient.
- Maximum.
- Observation count.

The arithmetic mean alone is insufficient for
latency-sensitive systems.

### 6.1 Percentile Interpretation

P99 represents a latency threshold associated with
approximately 99 percent of observations.

The remaining observations may be substantially slower.

High-percentile estimates require sufficient observations.

For example, 10,000 observations provide only about one
observation in the upper 0.01 percent of the distribution.

That is not sufficient for a robust characterization of
P99.99 across repeated workloads.

### 6.2 Histogram Resolution

Maxwell's initial histogram implementation uses fixed
integer bucket bounds.

Intermediate percentiles are upper-bound estimates based
on bucket placement, capped at the observed maximum.

Minimum, maximum, sum, and count are exact for accepted
observations.

Overflow observations may produce coarse tail estimates.

Bucket resolution should match the expected latency range.

### 6.3 Aggregation

Do not average independently calculated percentiles.

Aggregate compatible histogram observations first and
calculate percentiles from the combined distribution.

## 7. Benchmark Categories

### 7.1 Microbenchmarks

Measure isolated operations such as:

- Fixed-point arithmetic.
- Protocol encoding.
- Protocol decoding.
- Sequence validation.
- Price-level lookup.
- Book mutation.
- Queue insertion and removal.
- Subscription filtering.
- Publication fan-out.
- Histogram recording.

Microbenchmarks help identify local implementation costs.

They do not establish integrated system performance.

### 7.2 Component Benchmarks

Measure realistic workloads within a complete component.

Examples:

- Book updates across multiple instruments.
- Publisher fan-out across subscriber populations.
- Recovery replay across bounded buffers.
- Protocol decoding across mixed message types.

### 7.3 End-to-End Benchmarks

Measure the integrated application using:

- Simulator-generated market data.
- Recorded market data.
- Planck-generated market data.
- Multiple downstream consumers.
- Real transport configurations.

### 7.4 Stress Benchmarks

Exercise the system near and beyond its intended capacity.

Measure:

- Queue growth.
- Rejected operations.
- Subscriber invalidation.
- Recovery frequency.
- Memory consumption.
- Tail latency.
- Recovery after overload.

## 8. Workload Design

A benchmark workload should specify:

- Random seed.
- Instrument count.
- Event count.
- Event-type distribution.
- Book depth.
- Subscriber count.
- Subscription selectivity.
- Message sizes.
- Arrival-rate distribution.
- Burst behavior.
- Recovery scenarios.
- Slow-consumer behavior.

### 8.1 Proposed Benchmark Matrix

The following values are starting test configurations,
not established system capacity.

| Parameter | Proposed Values |
|-----------|-----------------|
| Instruments | 1, 100, 1,000 |
| Price levels per side | 10, 100, 1,000 |
| Subscribers | 1, 8, 32, 64 |
| Subscription selectivity | 1%, 10%, 100% |
| Input rate | 10k, 100k, 1M events/s |
| Burst multiplier | 2x, 5x, 10x |
| Queue occupancy | Empty, moderate, near-full |
| Recovery gaps | Small, medium, large |

Only configurations supported by the component capacity
limits should be used.

Increase workload size incrementally.

### 8.2 Open-Loop and Closed-Loop Workloads

A closed-loop generator waits for processing progress
before submitting more work.

An open-loop generator attempts to submit work according
to an independent arrival schedule.

Closed-loop tests are useful for measuring service cost.

Open-loop tests are important for measuring queueing and
overload behavior.

### 8.3 Coordinated Omission

A benchmark can underreport latency if it stops generating
work while the system is stalled.

This is known as coordinated omission.

Arrival schedules and latency measurements should account
for intended submission times when evaluating realistic
open-loop behavior.

## 9. Benchmark Environment

Record:

- CPU model.
- Physical core count.
- Logical processor count.
- Cache configuration.
- NUMA topology.
- Installed memory.
- Operating system.
- Kernel version.
- Rust compiler version.
- Cargo version.
- Build profile.
- Compiler flags.
- Git commit.
- Transport configuration.
- CPU affinity.
- Power management settings.
- Background workload.

Useful Linux commands:

```bash
uname -a

lscpu

free -h

rustc -Vv

cargo --version

git rev-parse HEAD

git status --short
```

Additional NUMA information, when the relevant tools
are installed:

```bash
numactl --hardware
```

Benchmarks executed inside virtual machines or WSL2
should be identified as such.

Their results should not automatically be compared with
dedicated bare-metal measurements.

## 10. Build Configuration

Use optimized builds for performance measurements.

```bash
cargo build --workspace --release
```

Inspect the existing workspace release profile before
changing optimization settings.

Potential experimental settings include:

```toml
[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
```

These settings are candidates for measurement, not
mandatory configuration changes.

Link-time optimization and code generation settings
can affect compilation time, binary size, and runtime
performance.

### 10.1 Native CPU Optimization

For local hardware-specific experiments:

```bash
RUSTFLAGS="-C target-cpu=native" \
    cargo build --release
```

Native CPU optimization can produce binaries that are
incompatible with other processors.

Production builds must target the intended deployment
hardware.

### 10.2 Profiling Symbols

A profiling build may retain debug information:

```toml
[profile.profiling]
inherits = "release"
debug = true
```

Build using:

```bash
cargo build --profile profiling
```

Record the selected profile when reporting results.

## 11. Timing Methodology

Use a monotonic clock for local elapsed-time measurement.

```rust
use std::time::Instant;

let start = Instant::now();

// Execute the measured operation.

let elapsed = start.elapsed();

println!("Elapsed: {} ns", elapsed.as_nanos());
```

This example illustrates timing boundaries.

It is not a complete benchmarking harness.

### 11.1 Measurement Overhead

Timing calls introduce overhead.

For extremely small operations, measure batches and
evaluate instrumentation overhead independently.

Avoid subtracting a fixed timing overhead without
validating the assumptions behind that adjustment.

### 11.2 Compiler Optimization

Microbenchmarks must prevent the compiler from eliminating
the measured work.

Rust provides std::hint::black_box as a best-effort
optimization barrier.

```rust
use std::hint::black_box;

let input = black_box(100u64);
let result = input.wrapping_mul(42);

black_box(result);
```

### 11.3 Cross-Host Timing

End-to-end timing across Planck and Maxwell hosts requires
a defined clock synchronization strategy.

Possible approaches include:

- PTP.
- Hardware timestamping.
- Appropriately characterized clock synchronization.
- Round-trip measurements when suitable.

Round-trip latency divided by two is not necessarily
accurate one-way latency.

## 12. Linux Profiling

### 12.1 Hardware Performance Counters

After the server application is available:

```bash
perf stat \
    -e cycles,instructions,cache-misses,branch-misses \
    -- ./target/release/maxwell-server
```

Available events depend on processor capabilities,
kernel configuration, and permissions.

### 12.2 CPU Profiling

```bash
perf record -g \
    -- ./target/release/maxwell-server

perf report
```

Profiling quality depends on symbol availability and
stack unwinding configuration.

### 12.3 System Call Inspection

```bash
strace -c ./target/release/maxwell-server
```

System call tracing can significantly alter timing.

Use it for diagnostics rather than authoritative latency
measurement.

### 12.4 Memory Inspection

```bash
/usr/bin/time -v ./target/release/maxwell-server
```

Useful observations include maximum resident memory,
page faults, and context switches.

Peak resident memory does not directly identify the
number of heap allocations.

## 13. Optimization Priorities

### 13.1 Algorithms

Evaluate algorithmic complexity before applying low-level
optimizations.

Examples include:

- Price-level lookup.
- Instrument routing.
- Subscriber lookup.
- Subscription filtering.
- Recovery buffer traversal.

### 13.2 Allocation

Reduce avoidable allocation through:

- Preallocated buffers.
- Bounded queues.
- Reusable scratch storage.
- Appropriate ownership transfer.
- Compact message representations.

### 13.3 Copying

Avoid unnecessary copying of large snapshots or payloads.

Copying small value types may be cheaper than introducing
shared ownership or additional indirection.

### 13.4 Cache Behavior

Evaluate:

- Data locality.
- Working-set size.
- Cache-line contention.
- Hot and cold field separation.
- Pointer indirection.

### 13.5 Branching

Measure branch behavior before replacing readable
conditional logic with complex branchless implementations.

### 13.6 Batching

Batching may improve throughput by amortizing:

- System calls.
- Queue synchronization.
- Protocol processing overhead.

Batching can increase latency for individual events.

Batch size and maximum waiting time must be evaluated
together.

## 14. Publisher Performance

Publisher cost depends on:

- Subscriber count.
- Subscription filter size.
- Filter selectivity.
- Payload size.
- Queue occupancy.
- Backpressure policy.

The initial publisher uses deterministic subscriber
ordering and sorted instrument filters.

Potential future optimizations include:

- Instrument-to-subscriber indexes.
- Event-type indexes.
- Specialized wildcard subscription handling.
- Shared immutable payload representations.

These optimizations require explicit memory bounds and
deterministic delivery semantics.

## 15. Recovery Performance

Recovery benchmarks should measure:

- Gap detection latency.
- Recovery request latency.
- Time awaiting recovery data.
- Snapshot application time.
- Buffered replay throughput.
- Time to synchronized state.
- Peak recovery buffer occupancy.

Normal steady-state performance and recovery performance
must be reported separately.

A system that processes ordinary events quickly but
recovers slowly may still be operationally unsuitable.

## 16. Metrics Overhead

Measure processing with instrumentation enabled and
disabled where a valid comparison is possible.

Evaluate:

- Counter recording.
- Histogram recording.
- Atomic counter contention.
- Snapshot generation.
- Histogram aggregation.
- Export formatting.

Metric export must not be performed synchronously for
every market event.

## 17. Benchmark Procedure

1. Record the environment and Git commit.
2. Build the selected optimized profile.
3. Establish the initial market state.
4. Warm up the relevant processing path.
5. Run a fixed reproducible workload.
6. Collect latency and throughput statistics.
7. Record errors, drops, and queue occupancy.
8. Repeat the experiment.
9. Compare variability across runs.
10. Validate resulting market state.
11. Save configuration and results.

Warm-up duration, run duration, and repetition count
must be documented.

## 18. Benchmark Report Template

```text
Benchmark:
Date:
Git commit:
Hardware:
Operating system:
Rust compiler:
Build profile:
Compiler flags:

Workload:
Random seed:
Instrument count:
Subscriber count:
Event distribution:
Message rate:
Run duration:
Warm-up duration:
Repetitions:

Measurement boundary:
Clock source:

Received messages:
Processed events:
Accepted publications:
Subscriber deliveries:
Rejected operations:
Discarded deliveries:
Sequence gaps:
Recovery events:

Throughput:
Minimum latency:
Mean latency:
P50:
P90:
P95:
P99:
P99.9:
P99.99:
Maximum latency:

Peak queue occupancy:
Peak resident memory:
CPU utilization:
Allocation measurements:

State validation result:
Observations:
Limitations:
```

## 19. Performance Regression Policy

A performance regression should be investigated when
repeated measurements show a meaningful degradation
under equivalent conditions.

Regression thresholds should be established after
baseline variability is understood.

Do not introduce arbitrary hard thresholds before
collecting reliable baseline measurements.

Performance comparisons must preserve equivalent:

- Workload.
- Configuration.
- Correctness guarantees.
- Measurement boundaries.
- Hardware conditions.

## 20. Optimization Review Checklist

- [ ] A measured bottleneck exists.
- [ ] The benchmark is reproducible.
- [ ] The optimization has a defined hypothesis.
- [ ] Correctness tests pass.
- [ ] Recovery semantics remain correct.
- [ ] Ordering guarantees remain correct.
- [ ] Memory consumption remains bounded.
- [ ] Tail latency is evaluated.
- [ ] Before-and-after results are recorded.
- [ ] Operational tradeoffs are documented.
