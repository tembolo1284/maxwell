//! Integration tests for concurrent instrumentation.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use maxwell_metrics::AtomicCounter;

const WORKERS: usize = 8;
const ITERATIONS: u64 = 50_000;

#[test]
fn concurrent_counter_preserves_all_updates() {
    let counter = AtomicCounter::new();

    thread::scope(|scope| {
        for _ in 0..WORKERS {
            let counter = &counter;

            scope.spawn(move || {
                for _ in 0..ITERATIONS {
                    counter.increment();
                }
            });
        }
    });

    assert_eq!(counter.value(), WORKERS as u64 * ITERATIONS);
}

#[test]
fn concurrent_weighted_additions_are_preserved() {
    let counter = AtomicCounter::new();

    thread::scope(|scope| {
        for worker in 1..=WORKERS {
            let counter = &counter;

            scope.spawn(move || {
                for _ in 0..ITERATIONS {
                    counter.add(worker as u64);
                }
            });
        }
    });

    let weights: u64 = (1..=WORKERS as u64).sum();

    assert_eq!(counter.value(), weights * ITERATIONS);
}

#[test]
fn interval_collection_does_not_lose_updates() {
    let counter = AtomicCounter::new();
    let remaining = AtomicUsize::new(WORKERS);

    let collected = thread::scope(|scope| {
        for _ in 0..WORKERS {
            let counter = &counter;
            let remaining = &remaining;

            scope.spawn(move || {
                for _ in 0..ITERATIONS {
                    counter.increment();
                }

                remaining.fetch_sub(1, Ordering::AcqRel);
            });
        }

        let mut total = 0u64;

        while remaining.load(Ordering::Acquire) != 0 {
            total += counter.take();

            thread::yield_now();
        }

        total += counter.take();

        total
    });

    assert_eq!(collected, WORKERS as u64 * ITERATIONS);

    assert_eq!(counter.value(), 0);
}

#[test]
fn concurrent_collectors_do_not_double_count() {
    let counter = AtomicCounter::with_value(100_000);

    let collected = thread::scope(|scope| {
        let mut handles = Vec::with_capacity(WORKERS);

        for _ in 0..WORKERS {
            let counter = &counter;

            handles.push(scope.spawn(move || counter.take()));
        }

        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .sum::<u64>()
    });

    assert_eq!(collected, 100_000);
    assert_eq!(counter.value(), 0);
}

#[test]
fn independent_atomic_counters_remain_independent() {
    let counters: [AtomicCounter; WORKERS] = std::array::from_fn(|_| AtomicCounter::new());

    thread::scope(|scope| {
        for counter in &counters {
            scope.spawn(move || {
                for _ in 0..ITERATIONS {
                    counter.increment();
                }
            });
        }
    });

    for counter in &counters {
        assert_eq!(counter.value(), ITERATIONS);
    }
}

#[test]
fn atomic_counter_never_wraps_under_contention() {
    let counter = AtomicCounter::with_value(u64::MAX - 1000);

    thread::scope(|scope| {
        for _ in 0..WORKERS {
            let counter = &counter;

            scope.spawn(move || {
                for _ in 0..ITERATIONS {
                    counter.increment();
                }
            });
        }
    });

    assert_eq!(counter.value(), u64::MAX);
}
