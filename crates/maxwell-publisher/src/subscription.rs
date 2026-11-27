//! Subscriber identity, lifecycle and queue ownership.

use std::fmt;

use crate::error::PublisherError;
use crate::event::Publication;
use crate::filter::SubscriptionFilter;
use crate::queue::BoundedQueue;

/// Stable subscriber identifier within a publisher instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct SubscriberId(u64);

impl SubscriberId {
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SubscriberId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Subscriber lifecycle.
///
/// PendingSnapshot:
///     Registered but not yet synchronized.
///
/// Active:
///     Eligible to receive matching publications.
///
/// ResyncRequired:
///     Incremental state is no longer valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriberState {
    PendingSnapshot,
    Active,
    ResyncRequired,
}

/// A single subscriber and its bounded publication queue.
#[derive(Debug)]
pub struct Subscription<I, P> {
    id: SubscriberId,
    state: SubscriberState,
    filter: SubscriptionFilter<I>,
    queue: BoundedQueue<Publication<I, P>>,
    watermark: Option<u64>,
}

impl<I: Ord, P> Subscription<I, P> {
    pub(crate) fn new(
        id: SubscriberId,
        filter: SubscriptionFilter<I>,
        queue_capacity: usize,
    ) -> Result<Self, PublisherError> {
        Ok(Self {
            id,
            state: SubscriberState::PendingSnapshot,
            filter,
            queue: BoundedQueue::new(queue_capacity)?,
            watermark: None,
        })
    }

    #[inline]
    pub fn id(&self) -> SubscriberId {
        self.id
    }

    #[inline]
    pub fn state(&self) -> SubscriberState {
        self.state
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.state == SubscriberState::Active
    }

    #[inline]
    pub fn filter(&self) -> &SubscriptionFilter<I> {
        &self.filter
    }

    /// Watermark of the externally established baseline.
    #[inline]
    pub fn watermark(&self) -> Option<u64> {
        self.watermark
    }

    #[inline]
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    #[inline]
    pub fn queue_capacity(&self) -> usize {
        self.queue.capacity()
    }

    #[inline]
    pub fn queue_is_full(&self) -> bool {
        self.queue.is_full()
    }

    #[inline]
    pub fn queue_is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn pending(&self) -> impl ExactSizeIterator<Item = &Publication<I, P>> {
        self.queue.iter()
    }

    #[inline]
    pub(crate) fn matches(&self, publication: &Publication<I, P>) -> bool {
        self.is_active()
            && self
                .filter
                .matches(publication.instrument.as_ref(), publication.kind)
    }

    #[inline]
    pub(crate) fn enqueue(
        &mut self,
        publication: Publication<I, P>,
    ) -> Result<(), Publication<I, P>> {
        self.queue.try_push(publication)
    }

    #[inline]
    pub(crate) fn pop(&mut self) -> Option<Publication<I, P>> {
        self.queue.pop()
    }

    #[inline]
    pub(crate) fn front(&self) -> Option<&Publication<I, P>> {
        self.queue.front()
    }

    /// Activate after the caller establishes a valid baseline.
    pub(crate) fn activate(&mut self, watermark: u64) {
        self.watermark = Some(watermark);
        self.state = SubscriberState::Active;
    }

    /// Invalidate the current baseline and queued increments.
    ///
    /// Returns the number of queued publications discarded.
    pub(crate) fn require_resync(&mut self) -> usize {
        self.state = SubscriberState::ResyncRequired;
        self.watermark = None;
        self.queue.clear()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventKind;

    fn subscription() -> Subscription<u32, u64> {
        Subscription::new(SubscriberId::new(1), SubscriptionFilter::all(), 4).unwrap()
    }

    #[test]
    fn subscriber_id_orders_numerically() {
        assert!(SubscriberId::new(1) < SubscriberId::new(2));
        assert_eq!(SubscriberId::new(42).get(), 42);
    }

    #[test]
    fn new_subscription_requires_baseline() {
        let subscription = subscription();

        assert_eq!(subscription.state(), SubscriberState::PendingSnapshot);

        assert!(!subscription.is_active());
        assert_eq!(subscription.watermark(), None);
    }

    #[test]
    fn activates_with_watermark() {
        let mut subscription = subscription();

        subscription.activate(100);

        assert!(subscription.is_active());
        assert_eq!(subscription.watermark(), Some(100));
    }

    #[test]
    fn inactive_subscriber_does_not_match() {
        let subscription = subscription();

        let publication = Publication::new(1, Some(42), EventKind::Trade, 1000, 10);

        assert!(!subscription.matches(&publication));
    }

    #[test]
    fn active_subscriber_matches() {
        let mut subscription = subscription();
        subscription.activate(0);

        let publication = Publication::new(1, Some(42), EventKind::Trade, 1000, 10);

        assert!(subscription.matches(&publication));
    }

    #[test]
    fn resync_invalidates_and_clears_queue() {
        let mut subscription = subscription();
        subscription.activate(0);

        subscription
            .enqueue(Publication::new(
                1,
                Some(42),
                EventKind::BookUpdate,
                1000,
                10,
            ))
            .unwrap();

        assert_eq!(subscription.require_resync(), 1);
        assert!(subscription.queue_is_empty());
        assert_eq!(subscription.watermark(), None);

        assert_eq!(subscription.state(), SubscriberState::ResyncRequired);
    }
}
