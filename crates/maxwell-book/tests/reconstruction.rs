
//! Order book reconstruction and market depth tests.

use maxwell_core::*;

use maxwell_book::{
    BookConfig,
    BookError,
    BookManager,
    BookState,
    BookUpdate,
    BookUpdateKind,
    Level1,
    Level2,
    Level3,
    OrderBook,
};

use maxwell_book::depth::DepthLevel;

fn feed() -> FeedId {
    FeedId::new(
        StreamId::new(1),
        SessionId::new(1),
    )
}

fn instrument(id: u32) -> Instrument {
    Instrument::new(InstrumentConfig {
        id: InstrumentId::new(id),
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
    BookConfig::new(4_096, 1_024, 8)
}

fn book() -> OrderBook {
    OrderBook::new(instrument(1), feed(), config()).unwrap()
}

fn update(sequence: u64, kind: BookUpdateKind) -> BookUpdate {
    BookUpdate::new(
        feed(),
        InstrumentId::new(1),
        SequenceNumber::new(sequence),
        kind,
    )
}

fn add(
    book: &mut OrderBook,
    sequence: u64,
    id: u64,
    side: Side,
    price: i64,
    quantity: u64,
) {
    book.apply(update(
        sequence,
        BookUpdateKind::Add {
            order_id: OrderId::new(id),
            side,
            price: Price::new(price),
            quantity: Quantity::new(quantity),
        },
    ))
    .unwrap();
}

#[test]
fn empty_book() {
    let book = book();

    assert!(book.is_empty());
    assert_eq!(book.best_bid(), None);
    assert_eq!(book.best_ask(), None);
    assert_eq!(book.last_sequence(), None);
    assert!(book.validate_consistency().is_ok());
}

#[test]
fn aggregate_orders_at_same_price() {
    let mut book = book();

    add(&mut book, 1, 100, Side::Buy, 10_000, 200);
    add(&mut book, 2, 101, Side::Buy, 10_000, 300);

    let (price, level) = book.best_bid().unwrap();

    assert_eq!(price, Price::new(10_000));
    assert_eq!(level.quantity(), Quantity::new(500));
    assert_eq!(level.order_count(), 2);
    assert_eq!(book.order_count(), 2);
    assert!(book.validate_consistency().is_ok());
}

#[test]
fn best_bid_and_offer() {
    let mut book = book();

    add(&mut book, 1, 1, Side::Buy, 9_990, 100);
    add(&mut book, 2, 2, Side::Buy, 10_000, 200);
    add(&mut book, 3, 3, Side::Sell, 10_010, 300);
    add(&mut book, 4, 4, Side::Sell, 10_020, 400);

    let bbo = Level1::from_book(&book);

    assert_eq!(bbo.bid.unwrap().price, Price::new(10_000));
    assert_eq!(bbo.ask.unwrap().price, Price::new(10_010));
    assert_eq!(bbo.spread(), Some(10));
    assert_eq!(bbo.midpoint_floor(), Some(Price::new(10_005)));
}

#[test]
fn partial_execution() {
    let mut book = book();

    add(&mut book, 1, 1, Side::Buy, 10_000, 500);

    book.apply(update(
        2,
        BookUpdateKind::Execute {
            order_id: OrderId::new(1),
            quantity: Quantity::new(200),
        },
    ))
    .unwrap();

    let order = book.orders().get(OrderId::new(1)).unwrap();

    assert_eq!(order.quantity(), Quantity::new(300));
    assert_eq!(book.best_bid().unwrap().1.order_count(), 1);
    assert_eq!(
        book.best_bid().unwrap().1.quantity(),
        Quantity::new(300),
    );

    assert!(book.validate_consistency().is_ok());
}

#[test]
fn complete_execution_removes_order() {
    let mut book = book();

    add(&mut book, 1, 1, Side::Buy, 10_000, 500);

    book.apply(update(
        2,
        BookUpdateKind::Execute {
            order_id: OrderId::new(1),
            quantity: Quantity::new(500),
        },
    ))
    .unwrap();

    assert!(book.is_empty());
    assert_eq!(book.best_bid(), None);
}

#[test]
fn delete_best_bid_promotes_next_level() {
    let mut book = book();

    add(&mut book, 1, 1, Side::Buy, 9_990, 100);
    add(&mut book, 2, 2, Side::Buy, 10_000, 200);

    book.apply(update(
        3,
        BookUpdateKind::Delete {
            order_id: OrderId::new(2),
        },
    ))
    .unwrap();

    assert_eq!(book.best_bid().unwrap().0, Price::new(9_990));
    assert_eq!(book.order_count(), 1);
}

#[test]
fn replace_order_at_new_price() {
    let mut book = book();

    add(&mut book, 1, 1, Side::Buy, 10_000, 100);

    book.apply(update(
        2,
        BookUpdateKind::Replace {
            order_id: OrderId::new(1),
            new_price: Price::new(10_005),
            new_quantity: Quantity::new(200),
        },
    ))
    .unwrap();

    let order = book.orders().get(OrderId::new(1)).unwrap();

    assert_eq!(order.price(), Price::new(10_005));
    assert_eq!(order.quantity(), Quantity::new(200));
    assert_eq!(order.priority_sequence(), SequenceNumber::new(2));

    assert!(book.bids().level(Price::new(10_000)).unwrap().is_empty());
    assert_eq!(book.best_bid().unwrap().0, Price::new(10_005));
    assert!(book.validate_consistency().is_ok());
}

#[test]
fn replace_order_at_same_price() {
    let mut book = book();

    add(&mut book, 1, 1, Side::Buy, 10_000, 100);
    add(&mut book, 2, 2, Side::Buy, 10_000, 200);

    book.apply(update(
        3,
        BookUpdateKind::Replace {
            order_id: OrderId::new(1),
            new_price: Price::new(10_000),
            new_quantity: Quantity::new(400),
        },
    ))
    .unwrap();

    let level = book.best_bid().unwrap().1;

    assert_eq!(level.quantity(), Quantity::new(600));
    assert_eq!(level.order_count(), 2);
    assert!(book.validate_consistency().is_ok());
}

#[test]
fn duplicate_order_requires_recovery() {
    let mut book = book();

    add(&mut book, 1, 1, Side::Buy, 10_000, 100);

    let result = book.apply(update(
        2,
        BookUpdateKind::Add {
            order_id: OrderId::new(1),
            side: Side::Buy,
            price: Price::new(10_000),
            quantity: Quantity::new(200),
        },
    ));

    assert_eq!(
        result,
        Err(BookError::DuplicateOrder(OrderId::new(1))),
    );

    assert_eq!(book.state(), BookState::RecoveryRequired);
    assert_eq!(book.order_count(), 1);
    assert_eq!(book.last_sequence(), Some(SequenceNumber::ONE));
    assert!(book.validate_consistency().is_ok());
}

#[test]
fn excessive_execution_does_not_mutate_book() {
    let mut book = book();

    add(&mut book, 1, 1, Side::Buy, 10_000, 100);

    assert_eq!(
        book.apply(update(
            2,
            BookUpdateKind::Execute {
                order_id: OrderId::new(1),
                quantity: Quantity::new(101),
            },
        )),
        Err(BookError::InvalidReduction),
    );

    assert_eq!(
        book.orders().get(OrderId::new(1)).unwrap().quantity(),
        Quantity::new(100),
    );

    assert_eq!(book.state(), BookState::RecoveryRequired);
    assert!(book.validate_consistency().is_ok());
}

#[test]
fn aggregate_overflow_does_not_mutate_book() {
    let mut book = book();

    add(&mut book, 1, 1, Side::Buy, 10_000, u64::MAX);

    let result = book.apply(update(
        2,
        BookUpdateKind::Add {
            order_id: OrderId::new(2),
            side: Side::Buy,
            price: Price::new(10_000),
            quantity: Quantity::ONE,
        },
    ));

    assert_eq!(result, Err(BookError::QuantityOverflow));
    assert_eq!(book.order_count(), 1);
    assert_eq!(book.best_bid().unwrap().1.quantity(), Quantity::MAX);
    assert!(book.validate_consistency().is_ok());
}

#[test]
fn instrument_sequences_need_not_be_contiguous() {
    let mut book = book();

    add(&mut book, 100, 1, Side::Buy, 10_000, 100);
    add(&mut book, 105, 2, Side::Sell, 10_010, 100);

    assert_eq!(
        book.last_sequence(),
        Some(SequenceNumber::new(105)),
    );
}

#[test]
fn sequence_regression_rejected() {
    let mut book = book();

    add(&mut book, 100, 1, Side::Buy, 10_000, 100);

    assert!(matches!(
        book.apply(update(99, BookUpdateKind::Observe)),
        Err(BookError::NonIncreasingSequence { .. }),
    ));
}

#[test]
fn observation_does_not_mutate_depth() {
    let mut book = book();

    add(&mut book, 1, 1, Side::Buy, 10_000, 100);

    book.apply(update(2, BookUpdateKind::Observe)).unwrap();

    assert_eq!(book.order_count(), 1);
    assert_eq!(book.last_sequence(), Some(SequenceNumber::new(2)));
}

#[test]
fn level2_depth_ordering() {
    let mut book = book();

    add(&mut book, 1, 1, Side::Buy, 9_990, 100);
    add(&mut book, 2, 2, Side::Buy, 10_000, 200);
    add(&mut book, 3, 3, Side::Buy, 9_980, 300);

    let depth = Level2::new(&book);
    let mut output = [DepthLevel::EMPTY; 2];

    assert_eq!(depth.write_depth(Side::Buy, &mut output), 2);

    assert_eq!(output[0].price, Price::new(10_000));
    assert_eq!(output[1].price, Price::new(9_990));

    assert_eq!(depth.cumulative_quantity(Side::Buy, 2), 300);
}

#[test]
fn level3_priority_ordering() {
    let mut book = book();

    add(&mut book, 1, 50, Side::Buy, 10_000, 100);
    add(&mut book, 2, 20, Side::Buy, 10_000, 200);
    add(&mut book, 3, 10, Side::Buy, 9_990, 300);

    let depth = Level3::new(&book);
    let orders = depth.sorted_side(Side::Buy).unwrap();

    let ids: Vec<_> = orders
        .iter()
        .map(|order| order.order_id().raw())
        .collect();

    assert_eq!(ids, vec![50, 20, 10]);
}

#[test]
fn instrument_isolation() {
    let mut manager = BookManager::new(config()).unwrap();

    manager.register(instrument(1), feed()).unwrap();
    manager.register(instrument(2), feed()).unwrap();

    manager.apply(update(
        1,
        BookUpdateKind::Add {
            order_id: OrderId::new(1),
            side: Side::Buy,
            price: Price::new(10_000),
            quantity: Quantity::new(100),
        },
    ))
    .unwrap();

    assert_eq!(
        manager.book(InstrumentId::new(1)).unwrap().order_count(),
        1,
    );

    assert!(
        manager.book(InstrumentId::new(2)).unwrap().is_empty()
    );
}

#[test]
fn feed_recovery_invalidates_all_associated_books() {
    let mut manager = BookManager::new(config()).unwrap();

    manager.register(instrument(1), feed()).unwrap();
    manager.register(instrument(2), feed()).unwrap();

    manager.require_feed_recovery(feed());

    for book in manager.iter() {
        assert_eq!(book.state(), BookState::RecoveryRequired);
    }
}

