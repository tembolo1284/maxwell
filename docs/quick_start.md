# Maxwell Quick Start

## 1. Purpose

This guide introduces the Maxwell development environment,
workspace validation, and operational command-line interface.

The initial applications support local simulation,
historical recording, journal inspection, validation,
replay, and throughput benchmarking.

## 2. Prerequisites

Recommended development environment:

- Linux, macOS, or WSL2.
- Current stable Rust toolchain supporting Rust 2024.
- Cargo.
- Git.
- rustfmt.
- Clippy.

Linux is recommended for future networking and performance
engineering work.

Docker, Terraform, and AWS CLI are not required for the
current local workflow.

## 3. Install Rust

If Rust is not already installed:

```bash
curl --proto '=https' --tlsv1.2 -sSf \
    https://sh.rustup.rs | sh
```

Load the Cargo environment:

```bash
source "$HOME/.cargo/env"
```

Install development components:

```bash
rustup update stable
rustup component add rustfmt clippy
```

Verify the environment:

```bash
rustc --version
cargo --version
rustup show
```

Record the toolchain version when reporting benchmark
results or reproducible build problems.

## 4. Clone the Repository

```bash
git clone https://github.com/tembolo1284/maxwell.git
cd maxwell
```

Inspect the workspace:

```bash
cargo metadata --no-deps --format-version 1
cargo tree --workspace
```

## 5. Build and Validate

Development build:

```bash
cargo build --workspace
```

Release build:

```bash
cargo build --workspace --release
```

Complete validation:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Release-mode tests:

```bash
cargo test --workspace --release
```

Generate API documentation:

```bash
cargo doc --workspace --no-deps --open
```

## 6. Install the Maxwell CLI

Build the executable:

```bash
cargo build --release -p maxwell-cli
```

Run directly from the workspace:

```bash
./target/release/maxwell help
```

Alternatively, install the executable:

```bash
cargo install --path apps/maxwell-cli
```

Ensure the Cargo binary directory is on PATH:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

Verify installation:

```bash
maxwell version
maxwell help
```

The remaining examples assume the installed executable
is available on PATH.

## 7. Available Commands

| Command | Purpose |
|---------|---------|
| help | Display usage |
| version | Display version |
| scenarios | List simulation scenarios |
| simulate | Execute a simulation |
| record | Record market events |
| inspect | Inspect journal metadata |
| validate | Validate journal contents |
| replay | Replay historical events |
| benchmark | Measure simulator throughput |

## 8. Run Your First Simulation

List predefined scenarios:

```bash
maxwell scenarios
```

Execute a deterministic smoke workload:

```bash
maxwell simulate \
    --scenario smoke \
    --events 1000 \
    --seed 42
```

The output includes:

- Selected scenario.
- Random seed.
- Generated and delivered events.
- Event-type counts.
- Resting order counts.
- Delivery fault statistics.
- Elapsed time and throughput.
- Completion status.

The same seed and configuration reproduce the logical
market workload.

Elapsed time and measured throughput may differ.

## 9. Record Historical Market Data

Create a local data directory:

```bash
mkdir -p data
```

Record a clean market stream:

```bash
maxwell record \
    --scenario smoke \
    --events 10000 \
    --seed 42 \
    --output data/market.mxj
```

The default recording mode is Authoritative.

An authoritative journal requires contiguous upstream
event ordering.

The output file must not already exist.

Use a different filename for subsequent recordings.

## 10. Configure Recording Durability

Flush buffered data periodically:

```bash
maxwell record \
    --scenario clean \
    --events 100000 \
    --seed 42 \
    --flush-every 1024 \
    --output data/flushed.mxj
```

Synchronize file data periodically:

```bash
maxwell record \
    --scenario clean \
    --events 100000 \
    --seed 42 \
    --sync-every 4096 \
    --durable \
    --output data/durable.mxj
```

The durability options have different meanings:

| Option | Meaning |
|--------|---------|
| --flush-every N | Flush after every N records |
| --sync-every N | Synchronize file data after every N records |
| --durable | Synchronize file data at successful completion |

Flush and sync intervals are mutually exclusive.

Flushing does not guarantee persistence across power loss.

File-data synchronization does not guarantee directory-entry
durability.

## 11. Inspect a Journal

```bash
maxwell inspect data/market.mxj
```

Inspection displays:

- Journal mode.
- Recording identifier.
- Creation timestamp.
- Stream and session identifiers.
- Maximum payload size.
- File size.

Inspection validates the header without scanning every
record.

Use the validate command for full journal validation.

## 12. Validate a Journal

```bash
maxwell validate data/market.mxj
```

Validation checks the complete journal for:

- Supported format version.
- Record lengths.
- Integrity checksums.
- Protocol decoding.
- Feed identity.
- Recording timestamp ordering.
- Authoritative sequence continuity.

Successful strict validation reports:

```text
Status: VALID
```

For a journal with an incomplete final record:

```bash
maxwell validate data/market.mxj \
    --allow-incomplete-tail
```

This option permits a valid prefix followed by an
incomplete final record.

It does not permit arbitrary corruption or checksum
failures.

## 13. Replay Historical Events

Replay the entire journal without wall-clock delays:

```bash
maxwell replay data/market.mxj \
    --speed unbounded
```

Require authoritative replay:

```bash
maxwell replay data/market.mxj \
    --authoritative \
    --speed unbounded
```

Virtual playback:

```bash
maxwell replay data/market.mxj \
    --speed virtual
```

Real-time playback:

```bash
maxwell replay data/market.mxj \
    --speed realtime
```

Ten-times accelerated playback:

```bash
maxwell replay data/market.mxj \
    --speed 10
```

Half-speed playback:

```bash
maxwell replay data/market.mxj \
    --speed 1/2
```

Playback speed affects timing rather than event identity
or sequence values.

The initial CLI replay sink prints or discards records.
It does not publish them to a remote server.

## 14. Filter and Seek

Replay events for one instrument:

```bash
maxwell replay data/market.mxj \
    --instrument 1 \
    --print-events
```

Replay an inclusive sequence range:

```bash
maxwell replay data/market.mxj \
    --from-sequence 100 \
    --to-sequence 200 \
    --print-events
```

Seek to an exact upstream sequence:

```bash
maxwell replay data/market.mxj \
    --seek-sequence 5000
```

Seek by relative recording timestamp:

```bash
maxwell replay data/market.mxj \
    --seek-time 1000000
```

Configure the processing batch size:

```bash
maxwell replay data/market.mxj \
    --batch-size 256
```

Disable index construction for sequential playback:

```bash
maxwell replay data/market.mxj \
    --no-index
```

Seeking requires an index.

Filtering and seeking cannot be combined with the
authoritative replay option.

A partial stream requires an appropriate market baseline
before it can reconstruct authoritative state.

## 15. Simulate Delivery Faults

Execute a faulty workload:

```bash
maxwell simulate \
    --scenario mixed-faults \
    --events 10000 \
    --seed 42
```

Record the original delivery order:

```bash
maxwell record \
    --scenario mixed-faults \
    --events 10000 \
    --seed 42 \
    --raw \
    --output data/faulty.mxj
```

Validate the raw capture:

```bash
maxwell validate data/faulty.mxj
```

Replay the captured arrival order:

```bash
maxwell replay data/faulty.mxj \
    --speed unbounded
```

Raw captures may contain gaps, duplicates, and reordered
upstream sequences.

A valid raw capture is not necessarily a complete
authoritative market stream.

## 16. Benchmark Simulator Throughput

Use a release executable:

```bash
cargo build --release -p maxwell-cli
```

Execute a reproducible benchmark:

```bash
./target/release/maxwell benchmark \
    --scenario high-throughput \
    --events 1000000 \
    --seed 42 \
    --iterations 5 \
    --warmup 2
```

The benchmark reports:

- Minimum throughput.
- Median throughput.
- Maximum throughput.
- Mean throughput.
- Aggregate throughput.
- Total measured events.
- Total measured execution time.

The measurement covers simulator generation and local
sink delivery.

It excludes simulator initialization, post-run consistency
validation, network communication, and journal persistence.

See [Performance](performance.md) for measurement guidance.

## 17. Working with Individual Packages

Test the order book:

```bash
cargo test -p maxwell-book
```

Test the simulator:

```bash
cargo test -p maxwell-simulator
```

Test historical persistence and replay:

```bash
cargo test -p maxwell-replay
```

Test the CLI:

```bash
cargo test -p maxwell-cli
```

Run strict Clippy for one package:

```bash
cargo clippy -p maxwell-cli --all-targets -- -D warnings
```

## 18. Troubleshooting

### Command Not Found

Verify installation:

```bash
ls "$HOME/.cargo/bin/maxwell"
echo "$PATH"
```

Alternatively, execute the binary directly from
target/release.

### Output File Already Exists

Recording deliberately refuses to overwrite an existing
journal.

Choose a new output filename.

### Unsupported Rust Features

Update the stable toolchain:

```bash
rustup update stable
```

Verify the selected toolchain:

```bash
rustup show
```

### Clippy Warnings

Run:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Correct the underlying warning rather than introducing
unnecessary lint suppressions.

### Invalid Journal

Run strict validation:

```bash
maxwell validate data/market.mxj
```

Preserve the original journal for investigation.

Do not modify historical data merely to make validation
succeed.

## 19. Recommended Daily Development Workflow

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Before a performance measurement:

```bash
cargo build --workspace --release
```

Before a milestone release:

```bash
cargo test --workspace --release
cargo doc --workspace --no-deps
```

## 20. Deployment Status

The current quick-start workflow is local.

Distributed producer-to-server communication, Docker
deployment, Terraform infrastructure, AWS integration,
and live Planck interoperability are planned for Phase III.

See [Roadmap](roadmap.md) for implementation sequencing.

## 21. Related Documentation

- [Architecture](architecture.md)
- [Crate Reference](crates.md)
- [Operations](operations.md)
- [Testing](testing.md)
- [Performance](performance.md)
- [Roadmap](roadmap.md)
