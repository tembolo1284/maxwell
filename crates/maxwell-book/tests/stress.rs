
//! Deterministic high-volume order book stress tests.
//!
//! These tests validate correctness rather than latency.

use std::collections::BTreeMap;

use maxwell_core::*;

use maxwell_book::{
    BookConfig,
    BookSnapshot,
    BookUpdate,
    BookUpdateKind,
    Level1,
    OrderBook,
};

fn feed() -> FeedId {
    FeedId::new(
        StreamId::new(1),
        SessionId::new(1),
    )
}

fn instrument() -> Instrument {
    Instrument::new(InstrumentConfig {
        id: InstrumentId::new(1),
        price_scale: 2,
        quantity_scale: 0,
        tick_size: Price::new(1),
        lot_size: Quantity::ONE,
        min_price: Price::new(9_000),
        max_price: Price::new(11_000),
        min_quantity: Quantity::ONE,
        max_quantity: Quantity::MAX,
    })
    .unwrap()
}

fn book() -> OrderBook {
    OrderBook::new(
        instrument(),
        feed(),
        BookConfig::new(4_096, 8_192, 1),
    )
    .unwrap()
}

fn apply(
    book: &mut OrderBook,
    sequence: &mut u64,
    kind: BookUpdateKind,
) {
    *sequence += 1;

    book.apply(BookUpdate::new(
        feed(),
        InstrumentId::new(1),
        SequenceNumber::new(*sequence),
        kind,
    ))
    .unwrap();
}

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    fn next(&mut self) -> u64 {
        let mut value = self.state;

        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;

        self.state = value;

        value
    }
}

// ============================================================
// One million book mutations
// ============================================================

#[test]
fn one_million_book_updates() {
    const CYCLES: u64 = 250_000;

    let mut book = book();
    let mut sequence = 0;

    for id in 1..=CYCLES {
        apply(
            &mut book,
            &mut sequence,
            BookUpdateKind::Add {
                order_id: OrderId::new(id),
                side: Side::Buy,
                price: Price::new(10_000),
                quantity: Quantity::new(1_000),
            },
        );

        apply(
            &mut book,
            &mut sequence,
            BookUpdateKind::Execute {
                order_id: OrderId::new(id),
                quantity: Quantity::new(250),
            },
        );

        apply(
            &mut book,
            &mut sequence,
            BookUpdateKind::Reduce {
                order_id: OrderId::new(id),
                quantity: Quantity::new(250),
            },
        );

        apply(
            &mut book,
            &mut sequence,
            BookUpdateKind::Delete {
                order_id: OrderId::new(id),
            },
        );

        assert!(book.is_empty());
    }

    assert_eq!(sequence, 1_000_000);
    assert_eq!(
        book.last_sequence(),
        Some(SequenceNumber::new(1_000_000)),
    );

    assert!(book.validate_consistency().is_ok());
}

// ============================================================
// Large simultaneous order population
// ============================================================

#[test]
fn thousands_of_simultaneous_orders() {
    const ORDERS: u64 = 4_096;

    let mut book = book();
    let mut sequence = 0;

    for id in 1..=ORDERS {
        let side = if id % 2 == 0 {
            Side::Buy
        } else {
            Side::Sell
        };

        let price = match side {
            Side::Buy => 10_000 - (id % 100) as i64,
            Side::Sell => 10_001 + (id % 100) as i64,
        };

        apply(
            &mut book,
            &mut sequence,
            BookUpdateKind::Add {
                order_id: OrderId::new(id),
                side,
                price: Price::new(price),
                quantity: Quantity::new(100),
            },
        );
    }

    assert_eq!(book.order_count(), ORDERS as usize);
    assert!(!book.is_crossed());
    assert!(book.validate_consistency().is_ok());

    for id in (1..=ORDERS).rev() {
        apply(
            &mut book,
            &mut sequence,
            BookUpdateKind::Delete {
                order_id: OrderId::new(id),
            },
        );
    }

    assert!(book.is_empty());
    assert_eq!(book.best_bid(), None);
    assert_eq!(book.best_ask(), None);
    assert!(book.validate_consistency().is_ok());
}

// ============================================================
// Independent reference model
// ============================================================

#[derive(Debug, Clone, Copy)]
struct ReferenceOrder {
    side: Side,
    price: i64,
    quantity: u64,
}

fn compare_reference(
    book: &OrderBook,
    reference: &BTreeMap<u64, ReferenceOrder>,
) {
    assert_eq!(book.order_count(), reference.len());

    let mut bids: BTreeMap<i64, (u128, u32)> = BTreeMap::new();
    let mut asks: BTreeMap<i64, (u128, u32)> = BTreeMap::new();

    for (&id, order) in reference {
        let actual = book.orders()
            .get(OrderId::new(id))
            .unwrap();

        assert_eq!(actual.side(), order.side);
        assert_eq!(actual.price().raw(), order.price);
        assert_eq!(actual.quantity().raw(), order.quantity);

        let levels = match order.side {
            Side::Buy => &mut bids,
            Side::Sell => &mut asks,
        };

        let entry = levels.entry(order.price).or_insert((0, 0));

        entry.0 += u128::from(order.quantity);
        entry.1 += 1;
    }

    assert_eq!(book.bids().active_levels(), bids.len());
    assert_eq!(book.asks().active_levels(), asks.len());

    let actual_bids: Vec<_> = book.bids()
        .iter()
        .map(|(price, level)| {
            (
                price.raw(),
                (
                    u128::from(level.quantity().raw()),
                    level.order_count(),
                ),
            )
        })
        .collect();

    let actual_asks: Vec<_> = book.asks()
        .iter()
        .map(|(price, level)| {
            (
                price.raw(),
                (
                    u128::from(level.quantity().raw()),
                    level.order_count(),
                ),
            )
        })
        .collect();

    let expected_bids: Vec<_> = bids.into_iter().rev().collect();
    let expected_asks: Vec<_> = asks.into_iter().collect();

    assert_eq!(actual_bids, expected_bids);
    assert_eq!(actual_asks, expected_asks);

    assert!(book.validate_consistency().is_ok());
}

#[test]
fn randomized_updates_against_reference_model() {
    const ITERATIONS: u64 = 100_000;
    const ID_SPACE: u64 = 2_048;

    let mut book = book();
    let mut reference = BTreeMap::new();
    let mut sequence = 0;
    let mut rng = XorShift64::new(0x1234_5678_ABCD_EF01);

    for iteration in 0..ITERATIONS {
        let id = rng.next() % ID_SPACE + 1;

        if let Some(existing) = reference
            .get(&id)
            .copied()
        {
            let existing: ReferenceOrder = existing;

            match rng.next() % 3 {
                0 => {
                    apply(
                        &mut book,
                        &mut sequence,
                        BookUpdateKind::Delete {
                            order_id: OrderId::new(id),
                        },
                    );

                    reference.remove(&id);
                }

                1 => {
                    let reduction =
                        rng.next() % existing.quantity + 1;

                    apply(
                        &mut book,
                        &mut sequence,
                        BookUpdateKind::Execute {
                            order_id: OrderId::new(id),
                            quantity: Quantity::new(reduction),
                        },
                    );

                    if reduction == existing.quantity {
                        reference.remove(&id);
                    } else {
                        reference.insert(
                            id,
                            ReferenceOrder {
                                quantity: existing.quantity - reduction,
                                ..existing
                            },
                        );
                    }
                }

                _ => {
                    let price = match existing.side {
                        Side::Buy => {
                            10_000 - (rng.next() % 100) as i64
                        }

                        Side::Sell => {
                            10_001 + (rng.next() % 100) as i64
                        }
                    };

                    let quantity = rng.next() % 1_000 + 1;

                    apply(
                        &mut book,
                        &mut sequence,
                        BookUpdateKind::Replace {
                            order_id: OrderId::new(id),
                            new_price: Price::new(price),
                            new_quantity: Quantity::new(quantity),
                        },
                    );

                    reference.insert(
                        id,
                        ReferenceOrder {
                            price,
                            quantity,
                            ..existing
                        },
                    );
                }
            }
        } else {
            let side = if rng.next() & 1 == 0 {
                Side::Buy
            } else {
                Side::Sell
            };

            let price = match side {
                Side::Buy => {
                    10_000 - (rng.next() % 100) as i64
                }

                Side::Sell => {
                    10_001 + (rng.next() % 100) as i64
                }
            };

            let quantity = rng.next() % 1_000 + 1;

            apply(
                &mut book,
                &mut sequence,
                BookUpdateKind::Add {
                    order_id: OrderId::new(id),
                    side,
                    price: Price::new(price),
                    quantity: Quantity::new(quantity),
                },
            );

            reference.insert(
                id,
                ReferenceOrder {
                    side,
                    price,
                    quantity,
                },
            );
        }

        if iteration % 1_000 == 0 {
            compare_reference(&book, &reference);
        }
    }

    compare_reference(&book, &reference);
}

// ============================================================
// Deterministic reconstruction
// ============================================================

#[test]
fn deterministic_replay() {
    fn run() -> BookSnapshot {
        let mut book = book();
        let mut sequence = 0;

        for id in 1..=1_000 {
            apply(
                &mut book,
                &mut sequence,
                BookUpdateKind::Add {
                    order_id: OrderId::new(id),
                    side: Side::Buy,
                    price: Price::new(10_000 - (id % 50) as i64),
                    quantity: Quantity::new(id * 10),
                },
            );
        }

        for id in (1..=1_000).step_by(3) {
            apply(
                &mut book,
                &mut sequence,
                BookUpdateKind::Delete {
                    order_id: OrderId::new(id),
                },
            );
        }

        BookSnapshot::capture(&book).unwrap()
    }

    assert_eq!(run(), run());
}

// ============================================================
// Repeated snapshot recovery
// ============================================================

#[test]
fn repeated_snapshot_roundtrips() {
    let mut book = book();
    let mut sequence = 0;

    for id in 1..=1_000 {
        apply(
            &mut book,
            &mut sequence,
            BookUpdateKind::Add {
                order_id: OrderId::new(id),
                side: Side::Buy,
                price: Price::new(10_000),
                quantity: Quantity::new(100),
            },
        );
    }

    let expected = BookSnapshot::capture(&book).unwrap();
    let expected_bbo = Level1::from_book(&book);

    for _ in 0..100 {
        let snapshot = BookSnapshot::capture(&book).unwrap();

        book = snapshot.restore(book.config()).unwrap();

        assert_eq!(
            BookSnapshot::capture(&book).unwrap(),
            expected,
        );

        assert_eq!(Level1::from_book(&book), expected_bbo);
    }
}

