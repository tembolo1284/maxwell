
//! Snapshot reconstruction and recovery tests.

use maxwell_core::*;

use maxwell_book::{
    BookConfig,
    BookError,
    BookSnapshot,
    BookState,
    BookUpdate,
    BookUpdateKind,
    Level1,
    OrderBook,
    RestingOrder,
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

fn config() -> BookConfig {
    BookConfig::new(4_096, 1_024, 4)
}

fn populated_book() -> OrderBook {
    let mut book = OrderBook::new(
        instrument(),
        feed(),
        config(),
    )
    .unwrap();

    let orders = [
        (50, Side::Buy, 10_000, 100),
        (20, Side::Buy, 10_000, 200),
        (10, Side::Buy, 9_990, 300),
        (40, Side::Sell, 10_010, 400),
        (30, Side::Sell, 10_020, 500),
    ];

    for (index, (id, side, price, quantity)) in
        orders.into_iter().enumerate()
    {
        book.apply(BookUpdate::new(
            feed(),
            InstrumentId::new(1),
            SequenceNumber::new(index as u64 + 1),
            BookUpdateKind::Add {
                order_id: OrderId::new(id),
                side,
                price: Price::new(price),
                quantity: Quantity::new(quantity),
            },
        ))
        .unwrap();
    }

    book
}

#[test]
fn snapshot_roundtrip() {
    let original = populated_book();

    let snapshot = BookSnapshot::capture(&original).unwrap();
    let restored = snapshot.restore(config()).unwrap();

    assert_eq!(original.order_count(), restored.order_count());
    assert_eq!(original.last_sequence(), restored.last_sequence());
    assert_eq!(original.feed(), restored.feed());

    assert_eq!(
        Level1::from_book(&original),
        Level1::from_book(&restored),
    );

    assert_eq!(
        BookSnapshot::capture(&restored).unwrap(),
        snapshot,
    );

    assert!(restored.validate_consistency().is_ok());
}

#[test]
fn snapshot_ordering_is_canonical() {
    let book = populated_book();
    let snapshot = BookSnapshot::capture(&book).unwrap();

    let ids: Vec<_> = snapshot.orders
        .iter()
        .map(|order| order.order_id().raw())
        .collect();

    assert_eq!(ids, vec![10, 20, 30, 40, 50]);
}

#[test]
fn identical_books_produce_identical_snapshots() {
    let first = populated_book();
    let second = populated_book();

    assert_eq!(
        BookSnapshot::capture(&first).unwrap(),
        BookSnapshot::capture(&second).unwrap(),
    );
}

#[test]
fn snapshot_installation_restores_live_state() {
    let mut book = populated_book();
    let snapshot = BookSnapshot::capture(&book).unwrap();

    book.require_recovery();

    assert_eq!(book.state(), BookState::RecoveryRequired);

    snapshot.install(&mut book).unwrap();

    assert_eq!(book.state(), BookState::Live);
    assert_eq!(book.order_count(), 5);
}

#[test]
fn invalid_snapshot_does_not_replace_existing_book() {
    let mut book = populated_book();

    let original = BookSnapshot::capture(&book).unwrap();
    let mut invalid = original.clone();

    invalid.orders.push(invalid.orders[0]);

    assert!(invalid.install(&mut book).is_err());

    assert_eq!(
        BookSnapshot::capture(&book).unwrap(),
        original,
    );
}

#[test]
fn future_priority_sequence_rejected() {
    let book = populated_book();
    let mut snapshot = BookSnapshot::capture(&book).unwrap();

    snapshot.orders.push(
        RestingOrder::new(
            OrderId::new(999),
            Side::Buy,
            Price::new(10_000),
            Quantity::new(100),
            SequenceNumber::new(999),
        )
        .unwrap(),
    );

    assert!(matches!(
        snapshot.restore(config()),
        Err(BookError::InvalidSnapshot),
    ));
}

#[test]
fn snapshot_without_watermark_rejected_if_nonempty() {
    let book = populated_book();
    let mut snapshot = BookSnapshot::capture(&book).unwrap();

    snapshot.last_sequence = None;

    assert!(matches!(
        snapshot.restore(config()),
        Err(BookError::InvalidSnapshot),
    ));
}

#[test]
fn snapshot_capacity_enforced() {
    let book = populated_book();
    let snapshot = BookSnapshot::capture(&book).unwrap();

    assert!(matches!(
        snapshot.restore(BookConfig::new(4_096, 2, 1)),
        Err(BookError::OrderCapacityExceeded),
    ));
}

#[test]
fn stale_snapshot_rejected() {
    let mut book = populated_book();
    let snapshot = BookSnapshot::capture(&book).unwrap();

    book.apply(BookUpdate::new(
        feed(),
        InstrumentId::new(1),
        SequenceNumber::new(6),
        BookUpdateKind::Observe,
    ))
    .unwrap();

    assert_eq!(
        snapshot.install(&mut book),
        Err(BookError::InvalidSnapshot),
    );

    assert_eq!(book.last_sequence(), Some(SequenceNumber::new(6)));
}

#[test]
fn stale_book_cannot_publish_authoritative_snapshot() {
    let mut book = populated_book();

    book.require_recovery();

    assert_eq!(
        BookSnapshot::capture(&book),
        Err(BookError::RecoveryRequired),
    );
}

#[test]
fn restored_book_accepts_incremental_updates() {
    let original = populated_book();
    let snapshot = BookSnapshot::capture(&original).unwrap();

    let mut restored = snapshot.restore(config()).unwrap();

    restored.apply(BookUpdate::new(
        feed(),
        InstrumentId::new(1),
        SequenceNumber::new(6),
        BookUpdateKind::Execute {
            order_id: OrderId::new(50),
            quantity: Quantity::new(50),
        },
    ))
    .unwrap();

    assert_eq!(
        restored.orders().get(OrderId::new(50)).unwrap().quantity(),
        Quantity::new(50),
    );

    assert!(restored.validate_consistency().is_ok());
}

