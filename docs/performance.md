# Maxwell Performance

## 1. Purpose

This document defines Maxwell's performance engineering
principles, benchmark methodology, measurement boundaries,
and optimization strategy.

Performance claims must be supported by reproducible
measurements.

Architectural intent alone does not establish achieved
throughput or latency.

## 2. Performance Objectives

- High sustained market data throughput.
- Low event processing latency.
- Predictable tail latency.
- Bounded memory consumption.
- Minimal unnecessary synchronization.
- Efficient binary encoding.
- Efficient market reconstruction.
- Predictable behavior under backpressure.
- Efficient historical persistence and replay.

Quantitative targets will be established through measured
baselines.

## 3. Performance Terminology

| Term | Meaning |
|------|---------|
| Throughput | Completed work per unit of time |
| Latency | Time required for an individual operation |
| Tail latency | High-percentile latency |
| Jitter | Variation in observed latency |
| Saturation | Workload exceeds sustainable capacity |
| Backpressure | Downstream capacity restricts progress |
| Goodput | Useful successfully completed work |
| Allocation rate | Dynamic allocations per unit of work |

Throughput and latency must be reported independently.

High aggregate throughput does not guarantee low
individual-event latency.

## 4. Measurement Boundaries

A benchmark must define where timing begins and ends.

Potential measurement boundaries include:

| Boundary | Measured Work |
|----------|---------------|
| Protocol | Encode or decode |
| Sequencing | Sequence validation |
| Book | Market mutation |
| Server | Integrated event processing |
| Publisher | Filtering and fan-out |
| Simulator | Generation and local delivery |
| Recording | Encoding and journal writing |
| Validation | Journal reading and verification |
| Replay | Historical record delivery |
| Network | Producer-to-consumer communication |
| End-to-end | Source event through downstream consumption |

Results from different boundaries must not be presented
as directly equivalent.

## 5. Throughput Metrics

Basic throughput:

    Throughput = Completed events / Elapsed seconds

Aggregate throughput across multiple measurements:

    Aggregate throughput =
        Sum of completed events / Sum of elapsed seconds

Aggregate throughput is not necessarily equivalent to
the arithmetic mean of individual throughput rates.

Report generated, accepted, and delivered event counts
separately when delivery faults or backpressure exist.

## 6. Latency Distributions

Latency should be reported using distributions rather
than averages alone.

Recommended measurements:

- Minimum.
- Median or p50.
- p90.
- p95.
- p99.
- p99.9.
- Maximum.
- Observation count.

Histogram resolution must be appropriate for the expected
latency range.

A histogram-derived percentile is an estimate determined
by its bucket boundaries.

A throughput benchmark does not establish these latency
percentiles unless individual latency observations are
actually collected.

## 7. Current Benchmark Implementation

The maxwell-cli application includes a simulator
throughput benchmark.

```bash
cargo build --release -p maxwell-cli

./target/release/maxwell benchmark \
    --scenario high-throughput \
    --events 1000000 \
    --seed 42 \
    --iterations 5 \
    --warmup 2
```

The benchmark uses:

- A deterministic scenario.
- A fixed seed.
- Independent simulator initialization per iteration.
- An unbounded pacing mode.
- A local accepting sink.
- Configurable warmup iterations.
- Configurable measured iterations.

Fault-injection scenarios are rejected for this baseline
throughput benchmark.

## 8. Benchmark Measurement Scope

The measured interval includes:

- Simulator event generation.
- Authoritative synthetic market updates.
- Local delivery processing.
- Simulator runtime bookkeeping.

The measured interval excludes:

- Simulator initialization.
- Post-run consistency validation.
- Network communication.
- Journal persistence.
- Reporting between iterations.

The benchmark is not a measurement of:

- Planck matching throughput.
- Maxwell server processing throughput.
- Network latency.
- End-to-end exchange latency.
- Durable journal throughput.

These require separate benchmarks.

## 9. Benchmark Reporting

The CLI reports:

- Minimum iteration throughput.
- Median iteration throughput.
- Maximum iteration throughput.
- Arithmetic mean throughput.
- Aggregate throughput.
- Total measured events.
- Total measured execution time.

Warmup iterations are excluded from aggregate results.

The same workload seed is reused across measured
iterations.

This controls workload variation but does not eliminate
operating-system or hardware variation.

## 10. Workload Design

Benchmark workloads should vary:

- Instrument count.
- Resting order count.
- Price-band width.
- Event distribution.
- Event rate.
- Order churn.
- Subscriber count.
- Subscription overlap.
- Recovery frequency.
- Journal durability policy.

Predefined simulator scenarios provide reproducible
starting points.

A single workload is insufficient to characterize
complete system performance.

## 11. Build Configuration

Use release builds for performance measurements.

```bash
cargo build --workspace --release
```

Record:

- Rust compiler version.
- Cargo version.
- Source commit.
- Build profile.
- Compiler flags.
- Target architecture.
- Enabled features.

Potential future optimization options include:

- Link-time optimization.
- Code generation unit configuration.
- Target CPU optimization.
- Profile-guided optimization.

Do not enable architecture-specific compiler flags in
portable distribution builds without compatibility review.

## 12. Benchmark Environment

Record:

- CPU model.
- Physical and logical core count.
- Memory capacity.
- Operating system.
- Kernel version.
- Virtualization environment.
- CPU frequency policy.
- NUMA topology where relevant.
- Storage device and filesystem.
- Network interface where relevant.

Avoid comparing results from substantially different
environments without identifying those differences.

Cloud instances may introduce additional scheduling
and resource variability.

## 13. Timing Methodology

Use monotonic clocks for elapsed-time measurement.

Do not subtract unsynchronized wall-clock timestamps
from different hosts.

Separate:

- Initialization time.
- Warmup time.
- Measured execution time.
- Validation time.
- Shutdown time.

For paced workloads, distinguish processing time from
intentional waiting.

For historical replay, distinguish index construction
time from playback execution time.

## 14. Linux Profiling

Basic hardware counter collection:

```bash
perf stat ./target/release/maxwell benchmark \
    --scenario high-throughput \
    --events 1000000 \
    --iterations 5 \
    --warmup 1
```

Collect a CPU profile:

```bash
perf record -g ./target/release/maxwell benchmark \
    --scenario high-throughput \
    --events 1000000 \
    --iterations 5
```

Inspect the profile:

```bash
perf report
```

Measure process resource usage:

```bash
/usr/bin/time -v ./target/release/maxwell benchmark \
    --scenario high-throughput \
    --events 1000000 \
    --iterations 5
```

Potential measurements include:

- CPU cycles.
- Instructions.
- Cache misses.
- Branch misses.
- Context switches.
- Page faults.
- Peak resident memory.

Profiler availability and permissions vary by platform.

## 15. Optimization Priorities

Recommended optimization order:

1. Establish a reproducible baseline.
2. Identify dominant execution costs.
3. Verify resource and allocation behavior.
4. Optimize the measured bottleneck.
5. Revalidate correctness.
6. Repeat the benchmark.
7. Record the result.

Avoid optimizing speculative bottlenecks.

## 16. Order Book Performance

Potential areas of investigation include:

- Dense price-level locality.
- Order registry lookup.
- Dynamic allocation.
- Price-level traversal.
- Best-price maintenance.
- Order count aggregation.
- Mutation batching.

The current logical order capacity does not guarantee
allocation-free registry operation.

A fixed-capacity order registry is a future candidate.

## 17. Publisher Performance

Publisher cost depends on:

- Subscriber count.
- Subscription filtering complexity.
- Publication size.
- Queue occupancy.
- Subscription overlap.
- Backpressure policy.

Benchmarks should distinguish:

- No matching subscribers.
- One matching subscriber.
- Multiple matching subscribers.
- Saturated subscriber queues.
- Subscriber invalidation.

## 18. Recovery Performance

Recovery benchmarks should measure:

- Gap detection.
- Missing-range construction.
- Buffered event insertion.
- Retransmission processing.
- Buffered event application.
- Synchronization restoration.
- Snapshot handling.

Recovery latency must be reported separately from
normal synchronized processing latency.

## 19. Recording Performance

Journal performance depends on:

- Encoded message size.
- Encoding cost.
- CRC32C calculation.
- Buffer size.
- Flush frequency.
- Synchronization frequency.
- Storage throughput.
- Filesystem behavior.

Compare durability configurations independently.

Example buffered recording:

```bash
maxwell record \
    --scenario high-throughput \
    --events 1000000 \
    --seed 42 \
    --output data/buffered.mxj
```

Example periodic synchronization:

```bash
maxwell record \
    --scenario high-throughput \
    --events 1000000 \
    --seed 42 \
    --sync-every 4096 \
    --durable \
    --output data/synchronized.mxj
```

Use distinct output files.

A buffered recording result must not be presented as
durable storage throughput.

## 20. Replay Performance

Replay performance depends on:

- Journal read throughput.
- Checksum verification.
- Protocol decoding.
- Index construction.
- Filtering.
- Playback pacing.
- Sink processing.
- Backpressure.

For sequential unbounded playback:

```bash
maxwell replay data/buffered.mxj \
    --speed unbounded \
    --no-index
```

For indexed playback:

```bash
maxwell replay data/buffered.mxj \
    --speed unbounded
```

Compare initialization and playback times separately.

Printing individual events introduces substantial
additional formatting and output costs.

Do not enable event printing for a baseline replay
throughput measurement.

## 21. Metrics Overhead

Instrumentation must be included in performance review.

Potential costs include:

- Atomic updates.
- Cache-coherence traffic.
- Histogram bucket selection.
- Timestamp collection.
- Metric snapshot construction.
- Exporter serialization.

Prefer owner-local instrumentation where practical.

Measure the difference between equivalent workloads
with different instrumentation configurations.

## 22. Future Network Benchmarks

Phase III should introduce measurements for:

- TCP stream ingestion.
- UDP datagram ingestion.
- Binary framing.
- Partial reads and writes.
- Producer-to-server throughput.
- Server-to-client throughput.
- Recovery under packet loss.
- Reconnection behavior.
- Cross-host latency.

Local loopback measurements must be distinguished from
physical or cloud network measurements.

Cross-host one-way latency requires appropriate clock
synchronization.

## 23. Benchmark Report Template

    Benchmark:
    Date:
    Source commit:
    Application version:
    Rust version:
    Build profile:
    Operating system:
    CPU:
    Memory:
    Storage:
    Scenario:
    Seed:
    Instrument count:
    Events per iteration:
    Warmup iterations:
    Measured iterations:
    Measurement boundary:

    Minimum throughput:
    Median throughput:
    Maximum throughput:
    Mean throughput:
    Aggregate throughput:

    p50 latency:
    p99 latency:
    p99.9 latency:

    Peak memory:
    Error count:
    Backpressure count:

    Observations:
    Limitations:
    Proposed improvements:

Leave unmeasured fields explicitly marked as unavailable.

## 24. Performance Regression Policy

A performance change should:

- Preserve logical correctness.
- Use an equivalent workload.
- Use comparable build configuration.
- Identify the measurement boundary.
- Report multiple observations.
- Distinguish measured improvement from speculation.

Do not introduce hard regression thresholds until
benchmark variability has been characterized.

## 25. Optimization Review Checklist

- Is the bottleneck measured?
- Is the workload reproducible?
- Is the measurement boundary documented?
- Are release builds used?
- Are resource limits preserved?
- Are correctness tests still passing?
- Are allocation claims verified?
- Are latency claims supported by observations?
- Are before-and-after results comparable?

## 26. Related Documentation

- [Architecture](architecture.md)
- [Concurrency](concurrency.md)
- [Testing](testing.md)
- [Operations](operations.md)
- [Roadmap](roadmap.md)
