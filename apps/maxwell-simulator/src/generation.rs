//! Public API integration tests for authoritative generation.

use std::collections::HashSet;

use maxwell_core::{
    EventEnvelope, FeedId, MarketEvent, SequenceNumber,
    SessionId, Side, StreamId,
};

use maxwell_simulator::config::{
    EventDistribution, FaultConfig, SimulatorConfig,
};
use maxwell_simulator::generator::EventGenerator;
use maxwell_simulator::scenario::Scenario;
use maxwell_simulator::simulator::{
    NullSink, RuntimeLimits, Simulator,
};
use maxwell_simulator::snapshot::MarketSnapshot;

// ============================================================
// Fixtures
// ============================================================

fn feed() -> FeedId {
    FeedId::new(
        StreamId::new(1),
        SessionId::new(1),
    )
}

fn config(event_count: u64) -> SimulatorConfig {
    let mut config = Scenario::Smoke.config().unwrap();

    config.run.event_count = event_count;

    config
}

fn collect_events(
    config: SimulatorConfig,
) -> Vec<EventEnvelope> {
    let mut generator =
        EventGenerator::new(config, feed()).unwrap();

    let mut events = Vec::new();

    while let Some(event) = generator.generate().unwrap() {
        events.push(event);
    }

    generator.market().validate_consistency().unwrap();

    events
}

// ============================================================
// Determinism
// ============================================================

#[test]
fn identical_seeds_produce_identical_event_streams() {
    let config = config(10_000);

    let first = collect_events(config);
    let second = collect_events(config);

    assert_eq!(first, second);
}

#[test]
fn different_seeds_produce_different_event_streams() {
    let mut first_config = config(1_000);
    let mut second_config = first_config;

    first_config.run.seed = 42;
    second_config.run.seed = 43;

    let first = collect_events(first_config);
    let second = collect_events(second_config);

    assert_ne!(first, second);
}

#[test]
fn different_fault_profiles_preserve_authoritative_stream() {
    let clean = config(10_000);

    let mut faulty = clean;

    faulty.faults = FaultConfig {
        drop_bps: 500,
        duplicate_bps: 1_000,
        reorder_bps: 2_000,
        reorder_window: 8,
    };

    assert_eq!(
        collect_events(clean),
        collect_events(faulty),
    );
}

// ============================================================
// Sequence and timestamp invariants
// ============================================================

#[test]
fn authoritative_sequences_are_contiguous() {
    let events = collect_events(config(10_000));

    for (index, event) in events.iter().enumerate() {
        assert_eq!(
            event.sequence(),
            SequenceNumber::new(index as u64 + 1),
        );

        assert_eq!(event.header.feed(), feed());
    }
}

#[test]
fn authoritative_exchange_timestamps_are_deterministic() {
    let mut config = config(1_000);

    config.run.start_timestamp_ns = 100_000;
    config.pacing.event_interval_ns = 250;

    let events = collect_events(config);

    for (index, event) in events.iter().enumerate() {
        let expected =
            100_000 + index as u64 * 250;

        assert_eq!(
            event.header.exchange_timestamp.unwrap().raw(),
            expected,
        );
    }
}

#[test]
fn every_generated_event_passes_stateless_validation() {
    for event in collect_events(config(10_000)) {
        event.event.validate().unwrap();
    }
}

// ============================================================
// Order identity and capacity
// ============================================================

#[test]
fn order_identifiers_are_never_reused() {
    let mut identifiers = HashSet::new();

    for event in collect_events(config(25_000)) {
        if let MarketEvent::OrderAdded(add) = event.event {
            assert!(
                identifiers.insert(add.order_id),
                "duplicate generated order identifier",
            );
        }
    }

    assert!(!identifiers.is_empty());
}

#[test]
fn additions_only_workload_respects_capacity() {
    let mut config = config(10_000);

    config.market.instrument_count = 1;
    config.market.max_orders_per_instrument = 4;

    config.distribution = EventDistribution {
        add: 10_000,
        modify: 0,
        cancel: 0,
        delete: 0,
        execute: 0,
    };

    let mut generator =
        EventGenerator::new(config, feed()).unwrap();

    let mut additions = 0;
    let mut deletions = 0;

    while let Some(event) = generator.generate().unwrap() {
        match event.event {
            MarketEvent::OrderAdded(_) => {
                additions += 1;
            }

            MarketEvent::OrderDeleted(_) => {
                deletions += 1;
            }

            other => {
                panic!("unexpected generated event: {other:?}");
            }
        }

        assert!(generator.market().total_orders() <= 4);
    }

    assert!(additions > 0);
    assert!(deletions > 0);

    assert_eq!(
        additions - deletions,
        generator.market().total_orders(),
    );

    generator.market().validate_consistency().unwrap();
}

#[test]
fn empty_market_falls_back_to_valid_addition() {
    let mut config = config(100);

    config.distribution = EventDistribution {
        add: 0,
        modify: 0,
        cancel: 0,
        delete: 10_000,
        execute: 0,
    };

    let mut generator =
        EventGenerator::new(config, feed()).unwrap();

    let event = generator.generate().unwrap().unwrap();

    assert!(
        matches!(event.event, MarketEvent::OrderAdded(_)),
    );
}

// ============================================================
// Fixed-point market invariants
// ============================================================

#[test]
fn generated_orders_respect_price_and_quantity_alignment() {
    let mut config = config(20_000);

    config.market.tick_size = 25;
    config.market.lot_size = 5;

    let mut generator =
        EventGenerator::new(config, feed()).unwrap();

    while generator.generate().unwrap().is_some() {}

    for market in generator.market().instruments() {
        for order in market.orders() {
            assert_eq!(
                order.price.raw() % config.market.tick_size,
                0,
            );

            assert_eq!(
                order.quantity.raw() % config.market.lot_size,
                0,
            );

            assert!(order.quantity.raw() > 0);

            market.instrument()
                .validate_limit_order(
                    order.price,
                    order.quantity,
                )
                .unwrap();
        }
    }

    generator.market().validate_consistency().unwrap();
}

#[test]
fn synthetic_resting_market_remains_noncrossed() {
    let config = config(25_000);

    let mut generator =
        EventGenerator::new(config, feed()).unwrap();

    while generator.generate().unwrap().is_some() {}

    for market in generator.market().instruments() {
        let mut best_bid = None;
        let mut best_ask = None;

        for order in market.orders() {
            match order.side {
                Side::Buy => {
                    best_bid = Some(
                        best_bid.map_or(
                            order.price,
                            |price: maxwell_core::Price| {
                                price.max(order.price)
                            },
                        ),
                    );
                }

                Side::Sell => {
                    best_ask = Some(
                        best_ask.map_or(
                            order.price,
                            |price: maxwell_core::Price| {
                                price.min(order.price)
                            },
                        ),
                    );
                }
            }
        }

        if let (Some(bid), Some(ask)) = (best_bid, best_ask) {
            assert!(bid < ask);
        }
    }
}

// ============================================================
// Long-running authoritative state
// ============================================================

#[test]
fn long_running_generation_preserves_market_consistency() {
    let mut config = config(100_000);

    config.market.instrument_count = 4;
    config.market.max_orders_per_instrument = 128;

    let mut generator =
        EventGenerator::new(config, feed()).unwrap();

    while generator.generate().unwrap().is_some() {
        if generator.generated().is_multiple_of(1_000) {
            generator.market()
                .validate_consistency()
                .unwrap();
        }
    }

    assert_eq!(generator.generated(), 100_000);

    assert_eq!(
        generator.market().next_expected().raw(),
        100_001,
    );

    assert!(
        generator.market().total_orders() <= 512,
    );

    generator.market().validate_consistency().unwrap();
}

// ============================================================
// Snapshot and history consistency
// ============================================================

#[test]
fn final_snapshot_is_reproducible() {
    let config = config(10_000);

    let mut first = Simulator::new(
        config,
        feed(),
        RuntimeLimits::default(),
    )
    .unwrap();

    let mut second = Simulator::new(
        config,
        feed(),
        RuntimeLimits::default(),
    )
    .unwrap();

    first.run_until_blocked(&mut NullSink).unwrap();
    second.run_until_blocked(&mut NullSink).unwrap();

    let first_snapshot =
        first.snapshot(SequenceNumber::new(10_000)).unwrap();

    let second_snapshot =
        second.snapshot(SequenceNumber::new(10_000)).unwrap();

    assert_eq!(first_snapshot, second_snapshot);
}

#[test]
fn snapshot_contains_every_authoritative_resting_order() {
    let mut generator =
        EventGenerator::new(config(10_000), feed()).unwrap();

    while generator.generate().unwrap().is_some() {}

    let snapshot = MarketSnapshot::capture(
        generator.market(),
        65_536,
    )
    .unwrap();

    assert_eq!(
        snapshot.instrument_count(),
        generator.market().instrument_count(),
    );

    assert_eq!(
        snapshot.order_count(),
        generator.market().total_orders(),
    );

    for market in generator.market().instruments() {
        let captured =
            snapshot.instrument(market.id()).unwrap();

        assert_eq!(captured.order_count(), market.len());

        for order in captured.orders() {
            assert_eq!(
                market.order(order.order_id),
                Some(order),
            );
        }
    }

    snapshot.validate_consistency().unwrap();
}

#[test]
fn retained_history_matches_original_generated_stream() {
    let config = config(10_000);

    let expected = collect_events(config);

    let mut simulator = Simulator::new(
        config,
        feed(),
        RuntimeLimits::default(),
    )
    .unwrap();

    simulator.run_until_blocked(&mut NullSink).unwrap();

    let retained: Vec<_> =
        simulator.history().iter().copied().collect();

    assert_eq!(retained, expected);
}

#[test]
fn processing_budgets_do_not_change_authoritative_state() {
    let config = config(10_000);

    let mut first = Simulator::new(
        config,
        feed(),
        RuntimeLimits {
            generation_budget: 1,
            delivery_budget: 1,
            ..RuntimeLimits::default()
        },
    )
    .unwrap();

    let mut second = Simulator::new(
        config,
        feed(),
        RuntimeLimits {
            generation_budget: 128,
            delivery_budget: 64,
            ..RuntimeLimits::default()
        },
    )
    .unwrap();

    first.run_until_blocked(&mut NullSink).unwrap();
    second.run_until_blocked(&mut NullSink).unwrap();

    assert!(
        first.history().iter().eq(second.history().iter()),
    );

    let first_snapshot =
        first.snapshot(SequenceNumber::new(10_000)).unwrap();

    let second_snapshot =
        second.snapshot(SequenceNumber::new(10_000)).unwrap();

    assert_eq!(first_snapshot, second_snapshot);

    first.validate_consistency().unwrap();
    second.validate_consistency().unwrap();
}
