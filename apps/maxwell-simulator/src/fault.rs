//! Deterministic delivery fault injection.
//!
//! Delivery faults never mutate authoritative market state.
//! Backpressure never consumes an input event or advances
//! the fault RNG.

use std::collections::VecDeque;

use maxwell_core::EventEnvelope;

use crate::config::FaultConfig;
use crate::error::{SimulatorError, SimulatorResult};
use crate::rng::{FAULT_STREAM, SplitMix64};

// ============================================================
// Fault classification
// ============================================================

/// Primary delivery decision for an authoritative event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaultAction {
    Pass,
    Drop,
    Duplicate,
    Reorder,
}

/// Result of accepting one authoritative event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultOutcome {
    pub action: FaultAction,

    /// Number of messages appended to the delivery queue.
    ///
    /// May include previously delayed events.
    pub enqueued: usize,

    /// Number of delayed events still retained.
    pub delayed: usize,
}

// ============================================================
// Fault statistics
// ============================================================

/// Cumulative fault injector statistics.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FaultStats {
    pub accepted: u64,
    pub passed: u64,
    pub dropped: u64,
    pub duplicated: u64,
    pub reordered: u64,
    pub released: u64,
    pub enqueued: u64,
    pub dequeued: u64,
    pub backpressure: u64,
}

// ============================================================
// Fault injector
// ============================================================

/// Bounded deterministic delivery fault injector.
///
/// Reordered events are withheld until a later event is
/// delivered or the reorder window reaches capacity.
#[derive(Debug)]
pub struct FaultInjector {
    config: FaultConfig,
    rng: SplitMix64,
    delayed: VecDeque<EventEnvelope>,
    outbound: VecDeque<EventEnvelope>,
    reorder_capacity: usize,
    delivery_capacity: usize,
    stats: FaultStats,
}

impl FaultInjector {
    pub fn new(
        config: FaultConfig,
        seed: u64,
        delivery_capacity: usize,
    ) -> SimulatorResult<Self> {
        config.validate()?;

        let reorder_capacity =
            config.effective_reorder_window();

        let immediate_capacity =
            if config.duplicate_bps != 0 { 2 } else { 1 };

        let required = reorder_capacity
            .checked_add(immediate_capacity)
            .ok_or(SimulatorError::ArithmeticOverflow(
                "fault injector delivery capacity",
            ))?;

        if delivery_capacity < required {
            return Err(SimulatorError::InvalidConfig(
                "delivery capacity is insufficient for faults",
            ));
        }

        let mut delayed = VecDeque::new();
        let mut outbound = VecDeque::new();

        delayed
            .try_reserve_exact(reorder_capacity)
            .map_err(|_| SimulatorError::AllocationFailed)?;

        outbound
            .try_reserve_exact(delivery_capacity)
            .map_err(|_| SimulatorError::AllocationFailed)?;

        Ok(Self {
            config,
            rng: SplitMix64::for_stream(seed, FAULT_STREAM),
            delayed,
            outbound,
            reorder_capacity,
            delivery_capacity,
            stats: FaultStats::default(),
        })
    }

    #[must_use]
    pub const fn config(&self) -> FaultConfig {
        self.config
    }

    #[must_use]
    pub const fn stats(&self) -> FaultStats {
        self.stats
    }

    #[must_use]
    pub const fn delivery_capacity(&self) -> usize {
        self.delivery_capacity
    }

    #[must_use]
    pub fn pending_delivery(&self) -> usize {
        self.outbound.len()
    }

    #[must_use]
    pub fn pending_delayed(&self) -> usize {
        self.delayed.len()
    }

    #[must_use]
    pub fn remaining_capacity(&self) -> usize {
        self.delivery_capacity - self.outbound.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.outbound.is_empty() && self.delayed.is_empty()
    }

    // --------------------------------------------------------
    // Fault selection
    // --------------------------------------------------------

    fn select_action(
        config: FaultConfig,
        rng: &mut SplitMix64,
    ) -> FaultAction {
        if !config.is_enabled() {
            return FaultAction::Pass;
        }

        let mut value = rng.basis_points();

        if value < config.drop_bps {
            return FaultAction::Drop;
        }

        value -= config.drop_bps;

        if value < config.duplicate_bps {
            return FaultAction::Duplicate;
        }

        value -= config.duplicate_bps;

        if value < config.reorder_bps {
            return FaultAction::Reorder;
        }

        FaultAction::Pass
    }

    // --------------------------------------------------------
    // Capacity preflight
    // --------------------------------------------------------

    fn required_capacity(
        &self,
        action: FaultAction,
    ) -> usize {
        match action {
            FaultAction::Drop => 0,

            FaultAction::Pass => {
                1 + self.delayed.len()
            }

            FaultAction::Duplicate => {
                2 + self.delayed.len()
            }

            FaultAction::Reorder => {
                if self.delayed.len() == self.reorder_capacity {
                    1 + self.delayed.len()
                } else {
                    0
                }
            }
        }
    }

    /// Returns whether the next input can be accepted.
    ///
    /// This previews the next random decision without
    /// consuming it.
    #[must_use]
    pub fn can_accept_next(&self) -> bool {
        let mut rng = self.rng;

        let action = Self::select_action(
            self.config,
            &mut rng,
        );

        self.required_capacity(action)
            <= self.remaining_capacity()
    }

    fn preflight(
        &mut self,
        required: usize,
    ) -> SimulatorResult<()> {
        let available = self.remaining_capacity();

        if required > available {
            self.stats.backpressure =
                self.stats.backpressure.saturating_add(1);

            return Err(SimulatorError::DeliveryBackpressure {
                required,
                available,
            });
        }

        Ok(())
    }

    // --------------------------------------------------------
    // Internal queue operations
    // --------------------------------------------------------

    fn enqueue(&mut self, event: EventEnvelope) {
        debug_assert!(
            self.outbound.len() < self.delivery_capacity
        );

        self.outbound.push_back(event);

        self.stats.enqueued =
            self.stats.enqueued.saturating_add(1);
    }

    fn release_delayed(&mut self) {
        while let Some(event) = self.delayed.pop_front() {
            self.enqueue(event);

            self.stats.released =
                self.stats.released.saturating_add(1);
        }
    }

    // --------------------------------------------------------
    // Authoritative event admission
    // --------------------------------------------------------

    /// Applies one deterministic delivery decision.
    ///
    /// On backpressure, the event remains unconsumed and
    /// the random decision remains unchanged for retry.
    pub fn accept(
        &mut self,
        event: EventEnvelope,
    ) -> SimulatorResult<FaultOutcome> {
        let mut rng = self.rng;

        let action = Self::select_action(
            self.config,
            &mut rng,
        );

        let required = self.required_capacity(action);

        self.preflight(required)?;

        let before = self.outbound.len();

        match action {
            FaultAction::Pass => {
                self.enqueue(event);
                self.release_delayed();

                self.stats.passed =
                    self.stats.passed.saturating_add(1);
            }

            FaultAction::Drop => {
                self.stats.dropped =
                    self.stats.dropped.saturating_add(1);
            }

            FaultAction::Duplicate => {
                self.enqueue(event);
                self.enqueue(event);
                self.release_delayed();

                self.stats.duplicated =
                    self.stats.duplicated.saturating_add(1);
            }

            FaultAction::Reorder => {
                if self.delayed.len() == self.reorder_capacity {
                    // Deliver the newer event before
                    // releasing the withheld older events.
                    self.enqueue(event);
                    self.release_delayed();
                } else {
                    self.delayed.push_back(event);
                }

                self.stats.reordered =
                    self.stats.reordered.saturating_add(1);
            }
        }

        self.rng = rng;

        self.stats.accepted =
            self.stats.accepted.saturating_add(1);

        Ok(FaultOutcome {
            action,
            enqueued: self.outbound.len() - before,
            delayed: self.delayed.len(),
        })
    }

    // --------------------------------------------------------
    // Outbound delivery
    // --------------------------------------------------------

    /// Observes the next outbound event without consuming it.
    #[must_use]
    pub fn front(&self) -> Option<&EventEnvelope> {
        self.outbound.front()
    }

    /// Removes the next successfully delivered event.
    pub fn pop_front(&mut self) -> Option<EventEnvelope> {
        let event = self.outbound.pop_front()?;

        self.stats.dequeued =
            self.stats.dequeued.saturating_add(1);

        Some(event)
    }

    /// Releases all remaining delayed events.
    ///
    /// Intended for end-of-run draining. The operation
    /// is atomic with respect to outbound capacity.
    pub fn flush_delayed(&mut self) -> SimulatorResult<usize> {
        let count = self.delayed.len();

        self.preflight(count)?;

        self.release_delayed();

        Ok(count)
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{
        EventHeader, EventId, FeedId, InstrumentId,
        MarketEvent, OrderAdded, OrderId, Price, Quantity,
        SequenceNumber, SessionId, Side, StreamId,
    };

    use crate::config::BASIS_POINTS;

    fn event(sequence: u64) -> EventEnvelope {
        EventEnvelope::new(
            EventHeader::new(
                EventId::new(
                    FeedId::new(
                        StreamId::new(1),
                        SessionId::new(1),
                    ),
                    SequenceNumber::new(sequence),
                ),
                InstrumentId::new(1),
                None,
            ),
            MarketEvent::OrderAdded(OrderAdded {
                order_id: OrderId::new(sequence),
                side: Side::Buy,
                price: Price::new(100),
                quantity: Quantity::new(10),
            }),
        )
    }

    fn drain(injector: &mut FaultInjector) -> Vec<u64> {
        let mut sequences = Vec::new();

        while let Some(event) = injector.pop_front() {
            sequences.push(event.sequence().raw());
        }

        sequences
    }

    #[test]
    fn disabled_faults_preserve_event_order() {
        let mut injector = FaultInjector::new(
            FaultConfig::disabled(),
            42,
            16,
        )
        .unwrap();

        for sequence in 1..=5 {
            let outcome =
                injector.accept(event(sequence)).unwrap();

            assert_eq!(outcome.action, FaultAction::Pass);
        }

        assert_eq!(
            drain(&mut injector),
            vec![1, 2, 3, 4, 5],
        );
    }

    #[test]
    fn full_drop_probability_discards_every_event() {
        let config = FaultConfig {
            drop_bps: BASIS_POINTS,
            ..FaultConfig::default()
        };

        let mut injector =
            FaultInjector::new(config, 42, 16).unwrap();

        for sequence in 1..=5 {
            let outcome =
                injector.accept(event(sequence)).unwrap();

            assert_eq!(outcome.action, FaultAction::Drop);
        }

        assert!(injector.is_empty());
        assert_eq!(injector.stats().dropped, 5);
        assert_eq!(injector.stats().accepted, 5);
    }

    #[test]
    fn full_duplicate_probability_duplicates_every_event() {
        let config = FaultConfig {
            duplicate_bps: BASIS_POINTS,
            ..FaultConfig::default()
        };

        let mut injector =
            FaultInjector::new(config, 42, 16).unwrap();

        for sequence in 1..=3 {
            injector.accept(event(sequence)).unwrap();
        }

        assert_eq!(
            drain(&mut injector),
            vec![1, 1, 2, 2, 3, 3],
        );

        assert_eq!(injector.stats().duplicated, 3);
        assert_eq!(injector.stats().enqueued, 6);
    }

    #[test]
    fn full_reorder_window_releases_newer_event_first() {
        let config = FaultConfig {
            reorder_bps: BASIS_POINTS,
            reorder_window: 2,
            ..FaultConfig::default()
        };

        let mut injector =
            FaultInjector::new(config, 42, 8).unwrap();

        injector.accept(event(1)).unwrap();
        injector.accept(event(2)).unwrap();

        assert_eq!(injector.pending_delayed(), 2);
        assert_eq!(injector.pending_delivery(), 0);

        injector.accept(event(3)).unwrap();

        assert_eq!(
            drain(&mut injector),
            vec![3, 1, 2],
        );

        assert_eq!(injector.pending_delayed(), 0);
        assert_eq!(injector.stats().released, 2);
    }

    #[test]
    fn delayed_events_can_be_flushed_at_end_of_run() {
        let config = FaultConfig {
            reorder_bps: BASIS_POINTS,
            reorder_window: 4,
            ..FaultConfig::default()
        };

        let mut injector =
            FaultInjector::new(config, 42, 8).unwrap();

        injector.accept(event(1)).unwrap();
        injector.accept(event(2)).unwrap();

        assert_eq!(injector.flush_delayed().unwrap(), 2);

        assert_eq!(
            drain(&mut injector),
            vec![1, 2],
        );

        assert!(injector.is_empty());
    }

    #[test]
    fn duplicate_delivery_is_atomic_under_backpressure() {
        let config = FaultConfig {
            duplicate_bps: BASIS_POINTS,
            ..FaultConfig::default()
        };

        let mut injector =
            FaultInjector::new(config, 42, 2).unwrap();

        injector.accept(event(1)).unwrap();

        injector.pop_front().unwrap();

        let rng_before = injector.rng;

        assert!(!injector.can_accept_next());

        assert!(matches!(
            injector.accept(event(2)),
            Err(SimulatorError::DeliveryBackpressure {
                required: 2,
                available: 1,
            }),
        ));

        assert_eq!(injector.rng, rng_before);
        assert_eq!(injector.pending_delivery(), 1);
        assert_eq!(injector.stats().accepted, 1);

        injector.pop_front().unwrap();

        assert!(injector.can_accept_next());

        injector.accept(event(2)).unwrap();

        assert_eq!(
            drain(&mut injector),
            vec![2, 2],
        );
    }

    #[test]
    fn failed_flush_preserves_delayed_events() {
        let config = FaultConfig {
            reorder_bps: BASIS_POINTS,
            reorder_window: 2,
            ..FaultConfig::default()
        };

        let mut injector =
            FaultInjector::new(config, 42, 3).unwrap();

        for sequence in 1..=4 {
            injector.accept(event(sequence)).unwrap();
        }

        assert_eq!(injector.pending_delivery(), 3);
        assert_eq!(injector.pending_delayed(), 1);

        assert!(injector.flush_delayed().is_err());

        assert_eq!(injector.pending_delayed(), 1);

        assert_eq!(
            drain(&mut injector),
            vec![3, 1, 2],
        );

        injector.flush_delayed().unwrap();

        assert_eq!(drain(&mut injector), vec![4]);
    }

    #[test]
    fn mixed_faults_are_reproducible() {
        let config = FaultConfig {
            drop_bps: 1_000,
            duplicate_bps: 1_000,
            reorder_bps: 1_000,
            reorder_window: 4,
        };

        let mut first =
            FaultInjector::new(config, 42, 16).unwrap();

        let mut second =
            FaultInjector::new(config, 42, 16).unwrap();

        for sequence in 1..=1_000 {
            assert_eq!(
                first.accept(event(sequence)).unwrap(),
                second.accept(event(sequence)).unwrap(),
            );

            assert_eq!(
                drain(&mut first),
                drain(&mut second),
            );
        }

        first.flush_delayed().unwrap();
        second.flush_delayed().unwrap();

        assert_eq!(
            drain(&mut first),
            drain(&mut second),
        );

        assert_eq!(first.stats(), second.stats());
    }
}
