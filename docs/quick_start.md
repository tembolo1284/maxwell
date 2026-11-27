# Maxwell Quick Start

## 1. Purpose

This guide describes how to establish a development environment,
build the Maxwell workspace, execute tests, and begin working
on individual components.

The library crates have an initial implementation.

The server, simulator, replay, and CLI applications remain
under development. Commands for those applications are
provided only when explicitly identified as planned.

## 2. Prerequisites

Required development tools:

- Git.
- Rust stable.
- Cargo.
- rustfmt.
- Clippy.
- A native linker and platform development tools.

Linux is the preferred environment for performance-oriented
development.

Windows through WSL2 and macOS may also be used where supported
by the relevant transport implementations.

The workspace manifests and any rust-toolchain.toml file
take precedence over this guide when determining toolchain
requirements.

## 3. Linux Environment

### 3.1 Ubuntu or Debian

Install the basic development packages:

```bash
sudo apt update

sudo apt install -y \
    build-essential \
    git \
    curl \
    pkg-config
```

Optional debugging and profiling packages:

```bash
sudo apt install -y \
    gdb \
    strace \
    valgrind
```

Linux perf installation varies by distribution and kernel.

Use the appropriate package for the running kernel.

### 3.2 Install Rust

Follow the official installation instructions:

https://rustup.rs/

After installation:

```bash
source "$HOME/.cargo/env"
```

Install development components:

```bash
rustup component add rustfmt clippy
```

Verify the environment:

```bash
rustc --version
cargo --version
rustup show
rustfmt --version
cargo clippy --version
```

Do not override a repository-pinned toolchain without
understanding the compatibility implications.

## 4. Clone the Repository

```bash
git clone https://github.com/tembolo1284/maxwell.git

cd maxwell
```

Inspect the workspace:

```bash
git status

ls

ls crates

cat Cargo.toml
```

Inspect recognized Cargo packages:

```bash
cargo metadata --no-deps --format-version 1
```

The workspace should contain the following library packages:

```text
maxwell-core
maxwell-protocol
maxwell-sequencer
maxwell-book
maxwell-transport
maxwell-recovery
maxwell-publisher
maxwell-gateway
maxwell-client
maxwell-metrics
```

## 5. Build the Workspace

### 5.1 Type Checking

```bash
cargo check --workspace
```

This is generally the fastest way to identify compilation
errors during development.

### 5.2 Development Build

```bash
cargo build --workspace
```

### 5.3 Release Build

```bash
cargo build --workspace --release
```

Development artifacts are normally located under:

```text
target/debug/
```

Release artifacts are normally located under:

```text
target/release/
```

The Cargo target directory may be overridden by local
configuration or environment variables.

Library builds do not necessarily produce executable
applications.

### 5.4 Individual Crate

```bash
cargo check -p maxwell-book

cargo build -p maxwell-book

cargo test -p maxwell-book
```

Cargo automatically builds the dependencies required by
the selected package.

## 6. Run Tests

### 6.1 Complete Workspace

```bash
cargo test --workspace
```

### 6.2 Individual Components

```bash
cargo test -p maxwell-core
cargo test -p maxwell-protocol
cargo test -p maxwell-sequencer
cargo test -p maxwell-book
cargo test -p maxwell-transport
cargo test -p maxwell-recovery
cargo test -p maxwell-publisher
cargo test -p maxwell-gateway
cargo test -p maxwell-client
cargo test -p maxwell-metrics
```

### 6.3 Release-Mode Tests

```bash
cargo test --workspace --release
```

### 6.4 Display Test Output

```bash
cargo test -p maxwell-book -- --nocapture
```

### 6.5 List Available Tests

```bash
cargo test -p maxwell-book -- --list
```

### 6.6 Run Matching Tests

```bash
cargo test -p maxwell-metrics histogram
```

The filter selects tests whose names contain the specified
string.

### 6.7 Run a Specific Integration Test Target

```bash
cargo test -p maxwell-metrics --test concurrency
```

Integration test target names correspond to files under
the package's tests directory.

### 6.8 Sequential Test Execution

```bash
cargo test --workspace -- --test-threads=1
```

Sequential execution can help diagnose interference between
tests that share external resources.

It is not a substitute for testing concurrent behavior.

## 7. Formatting and Static Analysis

Apply standard Rust formatting:

```bash
cargo fmt --all
```

Check formatting without modifying files:

```bash
cargo fmt --all -- --check
```

Run Clippy:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Run Clippy for one component:

```bash
cargo clippy -p maxwell-publisher --all-targets -- -D warnings
```

Warnings should be investigated rather than globally disabled.

Any narrowly scoped lint suppression should have a clear
technical justification.

## 8. Generate API Documentation

```bash
cargo doc --workspace --no-deps
```

Open generated documentation:

```bash
cargo doc --workspace --no-deps --open
```

Documentation is normally generated under:

```text
target/doc/
```

For individual packages:

```bash
cargo doc -p maxwell-recovery --no-deps --open
```

Generated Rust documentation is the preferred reference
for exact public API signatures.

## 9. First Example: Metrics

The following example demonstrates a standalone Maxwell
library component without requiring network connectivity
or a running matching engine.

Create:

```text
crates/maxwell-metrics/examples/basic.rs
```

```rust
use maxwell_metrics::{
    histogram::LATENCY_BOUNDS_NS,
    write_prometheus,
    MetricsRegistry,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut metrics = MetricsRegistry::<24>::new(4)?;

    let received = metrics.register_counter(
        "maxwell_messages_received_total",
    )?;

    let latency = metrics.register_histogram(
        "maxwell_processing_latency_nanoseconds",
        LATENCY_BOUNDS_NS,
    )?;

    for value in [100, 200, 300, 400, 500] {
        metrics.increment(received)?;
        metrics.record(latency, value)?;
    }

    let snapshot = metrics.histogram(latency)?.snapshot();

    println!("Observations: {}", snapshot.count);
    println!("Mean: {:?} ns", snapshot.mean());
    println!("P99: {:?} ns", snapshot.p99());

    let stdout = std::io::stdout();
    let mut writer = stdout.lock();

    write_prometheus(&mut writer, metrics.snapshots())?;

    Ok(())
}
```

Run the example:

```bash
cargo run -p maxwell-metrics --example basic
```

Expected summary:

```text
Observations: 5
Mean: Some(300) ns
P99: Some(500) ns
```

The remaining output contains Prometheus-compatible metric
samples.

The example uses synthetic latency values and does not
measure actual processing performance.

## 10. First Example: Publisher

Create:

```text
crates/maxwell-publisher/examples/basic.rs
```

```rust
use maxwell_publisher::{
    EventKind,
    Publisher,
    PublisherConfig,
    SubscriberId,
    SubscriptionFilter,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut publisher =
        Publisher::<u32, u64>::new(
            PublisherConfig::default(),
        )?;

    let subscriber = SubscriberId::new(1);

    publisher.subscribe(
        subscriber,
        SubscriptionFilter::all(),
    )?;

    let watermark = publisher.last_sequence();

    publisher.activate(subscriber, watermark)?;

    publisher.publish(
        Some(1001),
        EventKind::Trade,
        0,
        42,
    )?;

    while let Some(publication) =
        publisher.pop(subscriber)?
    {
        println!("{publication:?}");
    }

    Ok(())
}
```

Run:

```bash
cargo run -p maxwell-publisher --example basic
```

This example uses synthetic instrument identifiers and
payloads to demonstrate the generic publisher interface.

It is not a complete market data subscription protocol.

The activation step is appropriate for this empty synthetic
state. Real sessions require coordinated snapshot delivery.

## 11. Working with Individual Crates

Recommended workflow:

```bash
git checkout main

git pull origin main

git checkout -b feature/book-improvement
```

Inspect the relevant component:

```bash
ls crates/maxwell-book/src

ls crates/maxwell-book/tests

cat crates/maxwell-book/Cargo.toml
```

Make the required changes.

Run focused validation:

```bash
cargo fmt --all

cargo check -p maxwell-book

cargo test -p maxwell-book

cargo clippy -p maxwell-book \
    --all-targets -- -D warnings
```

Then validate the entire workspace:

```bash
cargo check --workspace

cargo test --workspace
```

Changes to a lower-level crate may affect several
downstream components.

## 12. Inspect Dependencies

Display the workspace dependency tree:

```bash
cargo tree --workspace
```

Display one package's dependencies:

```bash
cargo tree -p maxwell-client
```

Display reverse dependencies:

```bash
cargo tree -i maxwell-core
```

Identify duplicate dependency versions:

```bash
cargo tree --duplicates
```

The Cargo manifests and generated dependency tree are the
authoritative references for actual crate dependencies.

## 13. Debugging

### 13.1 Backtraces

```bash
RUST_BACKTRACE=1 cargo test -p maxwell-book
```

For a more detailed backtrace:

```bash
RUST_BACKTRACE=full cargo test -p maxwell-book
```

### 13.2 Inspect Compiler Diagnostics

```bash
cargo check -p maxwell-book
```

Resolve the earliest relevant compilation error before
investigating secondary errors.

### 13.3 Inspect Build Commands

```bash
cargo build -p maxwell-book -vv
```

### 13.4 Clean Build Artifacts

```bash
cargo clean
```

Cleaning the workspace removes generated build artifacts.

Use this when investigating stale artifacts or build
configuration problems, not as a routine response to
ordinary compilation errors.

## 14. Common Build Problems

### 14.1 Missing Workspace Package Metadata

Example:

```text
error inheriting repository from workspace root manifest
```

A crate may contain:

```toml
repository.workspace = true
```

The workspace root must then define the corresponding
metadata under its existing workspace.package section.

```toml
[workspace.package]
repository = "https://github.com/tembolo1284/maxwell"
```

Alternatively, remove the inherited field if repository
metadata is not required.

Do not create a duplicate workspace.package table.

### 14.2 Package Not Found

Example:

```text
package ID specification did not match any packages
```

Check:

- The package name in its Cargo.toml.
- Workspace member declarations.
- Workspace exclusion rules.
- The current working directory.

### 14.3 Unused Internal Methods

Example:

```text
warning: method is never used
```

Determine whether the method is:

- Required by upcoming integration.
- Accidentally unused.
- Redundant.
- Intended only for tests.

Remove redundant code or use a narrowly scoped lint
suppression with an appropriate explanation.

Do not make internal methods public solely to suppress
dead-code warnings.

### 14.4 Workspace Formatting Failure

cargo fmt requires valid Cargo manifests.

Resolve manifest parsing and workspace metadata errors
before retrying formatting.

### 14.5 Linker Not Found

On Ubuntu or Debian, verify that build-essential is
installed.

Other platforms require their corresponding native
development toolchain.

## 15. Planned Application Startup

The following package commands describe the intended
application entry points.

They will become available after the corresponding
packages are implemented and registered in the workspace.

```bash
cargo run -p maxwell-server --release

cargo run -p maxwell-simulator --release

cargo run -p maxwell-replay --release

cargo run -p maxwell-cli --release
```

Command-line arguments, configuration file formats,
network addresses, and startup ordering remain to be
finalized.

Do not assume these commands currently start a functioning
integrated system.

## 16. Recommended Daily Development Commands

```bash
cargo fmt --all

cargo check --workspace

cargo test --workspace

cargo clippy --workspace --all-targets -- -D warnings
```

Before submitting a significant change:

```bash
cargo fmt --all -- --check

cargo check --workspace

cargo clippy --workspace --all-targets -- -D warnings

cargo test --workspace

cargo test --workspace --release

cargo doc --workspace --no-deps
```

## 17. Developer Onboarding Checklist

- [ ] Clone the repository.
- [ ] Verify the Rust toolchain.
- [ ] Inspect the workspace manifests.
- [ ] Build the workspace.
- [ ] Run all workspace tests.
- [ ] Run Clippy.
- [ ] Generate Rust API documentation.
- [ ] Read the architecture document.
- [ ] Review the crate reference.
- [ ] Review the architectural decisions.
- [ ] Review the concurrency requirements.
- [ ] Identify the component being maintained.
- [ ] Review its unit and integration tests.
- [ ] Create a development branch.

## 18. Related Documentation

- [Architecture](architecture.md)
- [Crate Reference](crates.md)
- [Testing](testing.md)
- [Performance](performance.md)
- [Operations](operations.md)
