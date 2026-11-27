//! Single-owner deterministic market data publisher.

use crate::config::{BackpressurePolicy, PublisherConfig};
use crate::error::PublisherError;
use crate::event::{EventKind, Publication};
use crate::filter::SubscriptionFilter;
use crate::sink::{DrainReport, PublicationSink};
use crate::stats::PublisherStats;
use crate::subscription::{SubscriberId, SubscriberState, Subscription};

/// Result of an accepted publication.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PublishReport {
    pub sequence: u64,
    pub matched: usize,
    pub enqueued: usize,
    pub resync_required: usize,
    pub discarded: usize,
}

/// Deterministic single-owner publisher.
///
/// Subscribers are stored in ascending identifier order.
///
/// Instrument and payload types must be Copy to ensure that
/// fan-out does not invoke arbitrary cloning or allocation.
///
/// The publisher sequence is independent of exchange and
/// feed-specific sequence numbers.
///
/// The caller is responsible for:
///
/// - Validating and sequencing upstream market data.
/// - Applying events to authoritative book state.
/// - Establishing subscriber snapshots.
/// - Retaining rejected publications for retry.
/// - Managing downstream transport and concurrency.
#[derive(Debug)]
pub struct Publisher<I, P> {
    config: PublisherConfig,
    subscribers: Vec<Subscription<I, P>>,
    last_sequence: u64,
    stats: PublisherStats,
}

impl<I, P> Publisher<I, P>
where
    I: Copy + Ord,
    P: Copy,
{
    pub fn new(config: PublisherConfig) -> Result<Self, PublisherError> {
        config.validate()?;

        let mut subscribers = Vec::new();

        subscribers
            .try_reserve_exact(config.max_subscribers)
            .map_err(|_| PublisherError::AllocationFailed)?;

        Ok(Self {
            config,
            subscribers,
            last_sequence: 0,
            stats: PublisherStats::default(),
        })
    }

    #[inline]
    pub fn config(&self) -> &PublisherConfig {
        &self.config
    }

    #[inline]
    pub fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    #[inline]
    pub fn stats(&self) -> &PublisherStats {
        &self.stats
    }

    #[inline]
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }

    pub fn active_subscriber_count(&self) -> usize {
        self.subscribers
            .iter()
            .filter(|subscriber| subscriber.is_active())
            .count()
    }

    pub fn subscribers(&self) -> impl ExactSizeIterator<Item = &Subscription<I, P>> {
        self.subscribers.iter()
    }

    pub fn subscriber(&self, id: SubscriberId) -> Option<&Subscription<I, P>> {
        self.find_index(id)
            .ok()
            .map(|index| &self.subscribers[index])
    }

    #[inline]
    fn find_index(&self, id: SubscriberId) -> Result<usize, usize> {
        self.subscribers
            .binary_search_by_key(&id, |subscriber| subscriber.id())
    }

    #[inline]
    fn require_index(&self, id: SubscriberId) -> Result<usize, PublisherError> {
        self.find_index(id)
            .map_err(|_| PublisherError::SubscriberNotFound { subscriber_id: id })
    }

    #[inline]
    fn require_active_index(&self, id: SubscriberId) -> Result<usize, PublisherError> {
        let index = self.require_index(id)?;

        match self.subscribers[index].state() {
            SubscriberState::Active => Ok(index),

            SubscriberState::ResyncRequired => {
                Err(PublisherError::ResyncRequired { subscriber_id: id })
            }

            SubscriberState::PendingSnapshot => {
                Err(PublisherError::InvalidSubscriberState { subscriber_id: id })
            }
        }
    }

    /// Register a subscriber in PendingSnapshot state.
    ///
    /// Queue allocation occurs here, outside the publication
    /// hot path.
    pub fn subscribe(
        &mut self,
        id: SubscriberId,
        filter: SubscriptionFilter<I>,
    ) -> Result<(), PublisherError> {
        let insertion_index = match self.find_index(id) {
            Ok(_) => {
                return Err(PublisherError::SubscriberAlreadyExists { subscriber_id: id });
            }

            Err(index) => index,
        };

        if self.subscribers.len() >= self.config.max_subscribers {
            return Err(PublisherError::SubscriberLimitReached {
                limit: self.config.max_subscribers,
            });
        }

        if filter.instrument_count() > self.config.max_instruments_per_subscription {
            return Err(PublisherError::InstrumentLimitReached {
                limit: self.config.max_instruments_per_subscription,
            });
        }

        let subscription = Subscription::new(id, filter, self.config.queue_capacity)?;

        self.subscribers.insert(insertion_index, subscription);

        PublisherStats::increment(&mut self.stats.subscribers_registered);

        Ok(())
    }

    /// Remove a subscriber and discard its pending queue.
    ///
    /// Returns the number of discarded publications.
    pub fn unsubscribe(&mut self, id: SubscriberId) -> Result<usize, PublisherError> {
        let index = self.require_index(id)?;

        let subscription = self.subscribers.remove(index);
        let discarded = subscription.queue_len();

        PublisherStats::increment(&mut self.stats.subscribers_removed);

        PublisherStats::add(&mut self.stats.deliveries_discarded, discarded);

        Ok(discarded)
    }

    /// Activate a subscriber after establishing a snapshot
    /// at the current publisher watermark.
    ///
    /// The caller must ensure that the snapshot and subsequent
    /// incremental stream share a consistent delivery boundary.
    ///
    /// An older snapshot cannot be activated because this
    /// publisher does not retain a global replay history.
    pub fn activate(&mut self, id: SubscriberId, watermark: u64) -> Result<(), PublisherError> {
        let index = self.require_index(id)?;

        if self.subscribers[index].is_active() {
            return Err(PublisherError::InvalidSubscriberState { subscriber_id: id });
        }

        if watermark != self.last_sequence {
            return Err(PublisherError::InvalidWatermark {
                expected: self.last_sequence,
                received: watermark,
            });
        }

        self.subscribers[index].activate(watermark);

        PublisherStats::increment(&mut self.stats.subscribers_activated);

        Ok(())
    }

    /// Explicitly invalidate a subscriber.
    ///
    /// Useful when a downstream transport detects a failure
    /// or when the authoritative feed requires recovery.
    pub fn require_resync(&mut self, id: SubscriberId) -> Result<usize, PublisherError> {
        let index = self.require_index(id)?;

        Ok(self.invalidate_at(index))
    }

    /// Invalidate all subscribers.
    ///
    /// Returns the total number of queued publications discarded.
    pub fn require_resync_all(&mut self) -> usize {
        let mut discarded = 0usize;

        for index in 0..self.subscribers.len() {
            discarded = discarded.saturating_add(self.invalidate_at(index));
        }

        discarded
    }

    fn invalidate_at(&mut self, index: usize) -> usize {
        let subscriber = &mut self.subscribers[index];

        if subscriber.state() == SubscriberState::ResyncRequired {
            return 0;
        }

        let discarded = subscriber.require_resync();

        PublisherStats::increment(&mut self.stats.resync_transitions);

        PublisherStats::add(&mut self.stats.deliveries_discarded, discarded);

        discarded
    }

    /// Publish one validated market data event.
    ///
    /// RejectPublication:
    ///     Preflight all matching subscribers before changing
    ///     any queue or advancing the publication sequence.
    ///
    /// RequireResync:
    ///     Invalidate full matching subscribers and continue
    ///     delivering to healthy subscribers.
    ///
    /// A successful result consumes exactly one publication
    /// sequence, including events with no matching subscribers.
    pub fn publish(
        &mut self,
        instrument: Option<I>,
        kind: EventKind,
        timestamp_ns: u64,
        payload: P,
    ) -> Result<PublishReport, PublisherError> {
        let sequence = self
            .last_sequence
            .checked_add(1)
            .ok_or(PublisherError::SequenceExhausted)?;

        let publication = Publication::new(sequence, instrument, kind, timestamp_ns, payload);

        if self.config.backpressure_policy == BackpressurePolicy::RejectPublication {
            for subscriber in &self.subscribers {
                if subscriber.matches(&publication) && subscriber.queue_is_full() {
                    PublisherStats::increment(&mut self.stats.publications_rejected);

                    return Err(PublisherError::Backpressure {
                        subscriber_id: subscriber.id(),
                    });
                }
            }
        }

        let mut report = PublishReport {
            sequence,
            ..PublishReport::default()
        };

        for index in 0..self.subscribers.len() {
            if !self.subscribers[index].matches(&publication) {
                continue;
            }

            report.matched += 1;

            if self.subscribers[index].queue_is_full() {
                report.discarded += self.invalidate_at(index);
                report.resync_required += 1;

                PublisherStats::increment(&mut self.stats.deliveries_missed);

                continue;
            }

            let subscriber = &mut self.subscribers[index];

            // The queue was checked above. Single-owner
            // access prevents concurrent changes to its size.
            let result = subscriber.enqueue(publication);

            debug_assert!(result.is_ok(), "queue capacity invariant violated");

            if result.is_err() {
                // Defensive handling of an internal invariant
                // violation. Never silently lose an increment.
                report.discarded += self.invalidate_at(index);
                report.resync_required += 1;

                PublisherStats::increment(&mut self.stats.deliveries_missed);

                continue;
            }

            report.enqueued += 1;

            PublisherStats::increment(&mut self.stats.deliveries_enqueued);

            self.stats.observe_queue_depth(subscriber.queue_len());
        }

        self.last_sequence = sequence;

        PublisherStats::increment(&mut self.stats.publications_accepted);

        if report.matched == 0 {
            PublisherStats::increment(&mut self.stats.publications_unmatched);
        }

        Ok(report)
    }

    /// Inspect the oldest pending publication.
    ///
    /// Does not remove the publication from the queue.
    pub fn peek(&self, id: SubscriberId) -> Result<Option<&Publication<I, P>>, PublisherError> {
        let index = self.require_active_index(id)?;

        Ok(self.subscribers[index].front())
    }

    /// Transfer ownership of the oldest queued publication.
    ///
    /// The caller becomes responsible for delivery once the
    /// publication has been removed from the queue.
    pub fn pop(&mut self, id: SubscriberId) -> Result<Option<Publication<I, P>>, PublisherError> {
        let index = self.require_active_index(id)?;

        let publication = self.subscribers[index].pop();

        if publication.is_some() {
            PublisherStats::increment(&mut self.stats.deliveries_dequeued);
        }

        Ok(publication)
    }

    /// Drain at most `budget` publications into a nonblocking
    /// sink.
    ///
    /// Publications are removed only after successful sink
    /// acceptance.
    ///
    /// The first sink failure terminates the drain operation
    /// and leaves the failed publication queued for retry.
    pub fn drain<S>(
        &mut self,
        id: SubscriberId,
        sink: &mut S,
        budget: usize,
    ) -> Result<DrainReport<S::Error>, PublisherError>
    where
        S: PublicationSink<I, P>,
    {
        let index = self.require_active_index(id)?;

        let mut sent = 0usize;
        let mut error = None;

        while sent < budget {
            let subscriber = &mut self.subscribers[index];

            let Some(publication) = subscriber.front() else {
                break;
            };

            match sink.try_send(publication) {
                Ok(()) => {
                    let removed = subscriber.pop();

                    debug_assert!(removed.is_some());

                    sent += 1;

                    PublisherStats::increment(&mut self.stats.deliveries_dequeued);
                }

                Err(sink_error) => {
                    PublisherStats::increment(&mut self.stats.sink_failures);

                    error = Some(sink_error);
                    break;
                }
            }
        }

        Ok(DrainReport {
            sent,
            remaining: self.subscribers[index].queue_len(),
            error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestPublisher = Publisher<u32, u64>;

    fn publisher() -> TestPublisher {
        TestPublisher::new(PublisherConfig {
            max_subscribers: 4,
            queue_capacity: 4,
            max_instruments_per_subscription: 8,
            backpressure_policy: BackpressurePolicy::RequireResync,
        })
        .unwrap()
    }

    fn add_subscriber(publisher: &mut TestPublisher, id: u64) {
        let id = SubscriberId::new(id);

        publisher.subscribe(id, SubscriptionFilter::all()).unwrap();

        publisher.activate(id, publisher.last_sequence()).unwrap();
    }

    #[test]
    fn starts_with_zero_sequence() {
        let publisher = publisher();

        assert_eq!(publisher.last_sequence(), 0);
        assert_eq!(publisher.subscriber_count(), 0);
    }

    #[test]
    fn rejects_duplicate_subscriber() {
        let mut publisher = publisher();

        add_subscriber(&mut publisher, 1);

        assert_eq!(
            publisher.subscribe(SubscriberId::new(1), SubscriptionFilter::all(),),
            Err(PublisherError::SubscriberAlreadyExists {
                subscriber_id: SubscriberId::new(1),
            })
        );
    }

    #[test]
    fn enforces_subscriber_limit() {
        let mut publisher = publisher();

        for id in 1..=4 {
            add_subscriber(&mut publisher, id);
        }

        assert_eq!(
            publisher.subscribe(SubscriberId::new(5), SubscriptionFilter::all(),),
            Err(PublisherError::SubscriberLimitReached { limit: 4 })
        );
    }

    #[test]
    fn enforces_instrument_limit() {
        let mut publisher = publisher();

        let filter = SubscriptionFilter::for_instruments(&[1, 2, 3, 4, 5, 6, 7, 8, 9], 9).unwrap();

        assert_eq!(
            publisher.subscribe(SubscriberId::new(1), filter,),
            Err(PublisherError::InstrumentLimitReached { limit: 8 })
        );
    }

    #[test]
    fn maintains_sorted_subscribers() {
        let mut publisher = publisher();

        for id in [4, 1, 3, 2] {
            add_subscriber(&mut publisher, id);
        }

        let ids: Vec<_> = publisher
            .subscribers()
            .map(|subscriber| subscriber.id().get())
            .collect();

        assert_eq!(ids, vec![1, 2, 3, 4]);
    }

    #[test]
    fn rejects_stale_activation_watermark() {
        let mut publisher = publisher();
        let id = SubscriberId::new(1);

        publisher.subscribe(id, SubscriptionFilter::all()).unwrap();

        publisher
            .publish(Some(42), EventKind::Trade, 100, 10)
            .unwrap();

        assert_eq!(
            publisher.activate(id, 0),
            Err(PublisherError::InvalidWatermark {
                expected: 1,
                received: 0,
            })
        );
    }

    #[test]
    fn rejects_activation_of_active_subscriber() {
        let mut publisher = publisher();

        add_subscriber(&mut publisher, 1);

        assert_eq!(
            publisher.activate(SubscriberId::new(1), 0),
            Err(PublisherError::InvalidSubscriberState {
                subscriber_id: SubscriberId::new(1),
            })
        );
    }

    #[test]
    fn pending_subscriber_receives_no_events() {
        let mut publisher = publisher();
        let id = SubscriberId::new(1);

        publisher.subscribe(id, SubscriptionFilter::all()).unwrap();

        let report = publisher
            .publish(Some(42), EventKind::Trade, 100, 10)
            .unwrap();

        assert_eq!(report.matched, 0);
        assert_eq!(report.enqueued, 0);
        assert_eq!(publisher.last_sequence(), 1);
        assert_eq!(publisher.subscriber(id).unwrap().queue_len(), 0);
    }

    #[test]
    fn publishes_to_active_subscriber() {
        let mut publisher = publisher();

        add_subscriber(&mut publisher, 1);

        let report = publisher
            .publish(Some(42), EventKind::Trade, 100, 10)
            .unwrap();

        assert_eq!(report.sequence, 1);
        assert_eq!(report.matched, 1);
        assert_eq!(report.enqueued, 1);

        let publication = publisher.pop(SubscriberId::new(1)).unwrap().unwrap();

        assert_eq!(publication.sequence, 1);
        assert_eq!(publication.payload, 10);
    }

    #[test]
    fn unmatched_publication_advances_sequence() {
        let mut publisher = publisher();

        let report = publisher
            .publish(Some(42), EventKind::Trade, 100, 10)
            .unwrap();

        assert_eq!(report.sequence, 1);
        assert_eq!(report.matched, 0);
        assert_eq!(publisher.stats().publications_unmatched, 1);
    }

    #[test]
    fn sequence_exhaustion_does_not_wrap() {
        let mut publisher = publisher();

        publisher.last_sequence = u64::MAX;

        assert_eq!(
            publisher.publish(Some(42), EventKind::Trade, 100, 10,),
            Err(PublisherError::SequenceExhausted)
        );

        assert_eq!(publisher.last_sequence(), u64::MAX);
    }

    #[test]
    fn unsubscribe_discards_pending_events() {
        let mut publisher = publisher();

        add_subscriber(&mut publisher, 1);

        publisher
            .publish(Some(42), EventKind::Trade, 100, 10)
            .unwrap();

        assert_eq!(publisher.unsubscribe(SubscriberId::new(1)), Ok(1));

        assert_eq!(publisher.subscriber_count(), 0);
        assert_eq!(publisher.stats().deliveries_discarded, 1);
        assert_eq!(publisher.stats().pending_deliveries(), 0);
    }

    #[test]
    fn explicit_resync_is_idempotent() {
        let mut publisher = publisher();

        add_subscriber(&mut publisher, 1);

        publisher
            .publish(Some(42), EventKind::Trade, 100, 10)
            .unwrap();

        let id = SubscriberId::new(1);

        assert_eq!(publisher.require_resync(id), Ok(1));
        assert_eq!(publisher.require_resync(id), Ok(0));
        assert_eq!(publisher.stats().resync_transitions, 1);
    }

    #[test]
    fn invalidates_all_subscribers() {
        let mut publisher = publisher();

        add_subscriber(&mut publisher, 1);
        add_subscriber(&mut publisher, 2);

        publisher
            .publish(Some(42), EventKind::Trade, 100, 10)
            .unwrap();

        assert_eq!(publisher.require_resync_all(), 2);
        assert_eq!(publisher.active_subscriber_count(), 0);
        assert_eq!(publisher.stats().pending_deliveries(), 0);
    }

    #[test]
    fn unknown_subscriber_returns_error() {
        let mut publisher = publisher();
        let id = SubscriberId::new(999);

        assert_eq!(
            publisher.pop(id),
            Err(PublisherError::SubscriberNotFound { subscriber_id: id })
        );
    }
}
