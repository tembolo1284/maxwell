//! Deterministic instrument and event-kind filtering.

use crate::error::PublisherError;
use crate::event::EventKind;

/// A subscription filter.
///
/// Instrument identifiers are maintained in sorted order to
/// support deterministic binary-search lookup.
///
/// An empty instrument list is distinct from a wildcard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionFilter<I> {
    instruments: Vec<I>,
    all_instruments: bool,
    event_mask: u16,
    include_global: bool,
}

impl<I: Ord> SubscriptionFilter<I> {
    const ALL_EVENTS_MASK: u16 = (1u16 << EventKind::COUNT) - 1;

    /// Subscribe to all instruments and event kinds.
    pub fn all() -> Self {
        Self {
            instruments: Vec::new(),
            all_instruments: true,
            event_mask: Self::ALL_EVENTS_MASK,
            include_global: true,
        }
    }

    /// Subscribe to no instruments or event kinds.
    pub fn none() -> Self {
        Self {
            instruments: Vec::new(),
            all_instruments: false,
            event_mask: 0,
            include_global: false,
        }
    }

    /// Construct a filter for a bounded instrument list.
    pub fn for_instruments(instruments: &[I], limit: usize) -> Result<Self, PublisherError>
    where
        I: Copy,
    {
        if instruments.len() > limit {
            return Err(PublisherError::InstrumentLimitReached { limit });
        }

        let mut sorted = Vec::new();

        sorted
            .try_reserve_exact(instruments.len())
            .map_err(|_| PublisherError::AllocationFailed)?;

        sorted.extend_from_slice(instruments);
        sorted.sort_unstable();
        sorted.dedup();

        Ok(Self {
            instruments: sorted,
            all_instruments: false,
            event_mask: Self::ALL_EVENTS_MASK,
            include_global: true,
        })
    }

    /// Replace the event-kind mask.
    pub fn with_event_kinds(mut self, kinds: &[EventKind]) -> Self {
        self.event_mask = kinds.iter().fold(0u16, |mask, kind| mask | kind.bit());

        self
    }

    /// Configure whether instrument-independent events match.
    pub fn with_global_events(mut self, enabled: bool) -> Self {
        self.include_global = enabled;
        self
    }

    #[inline]
    pub fn matches(&self, instrument: Option<&I>, kind: EventKind) -> bool {
        if self.event_mask & kind.bit() == 0 {
            return false;
        }

        match instrument {
            None => self.include_global,

            Some(instrument) => {
                self.all_instruments || self.instruments.binary_search(instrument).is_ok()
            }
        }
    }

    #[inline]
    pub fn instrument_count(&self) -> usize {
        self.instruments.len()
    }

    #[inline]
    pub fn instruments(&self) -> &[I] {
        &self.instruments
    }

    #[inline]
    pub fn includes_all_instruments(&self) -> bool {
        self.all_instruments
    }

    #[inline]
    pub fn includes_global_events(&self) -> bool {
        self.include_global
    }

    #[inline]
    pub fn event_mask(&self) -> u16 {
        self.event_mask
    }
}

impl<I: Ord> Default for SubscriptionFilter<I> {
    fn default() -> Self {
        Self::all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matches_every_instrument() {
        let filter = SubscriptionFilter::<u32>::all();

        assert!(filter.matches(Some(&1), EventKind::Trade));
        assert!(filter.matches(Some(&999), EventKind::Quote));
        assert!(filter.matches(None, EventKind::Heartbeat));
    }

    #[test]
    fn empty_filter_matches_nothing() {
        let filter = SubscriptionFilter::<u32>::none();

        assert!(!filter.matches(Some(&1), EventKind::Trade));
        assert!(!filter.matches(None, EventKind::Heartbeat));
    }

    #[test]
    fn instrument_filter_sorts_and_deduplicates() {
        let filter = SubscriptionFilter::for_instruments(&[9u32, 2, 5, 2, 9], 10).unwrap();

        assert_eq!(filter.instruments(), &[2, 5, 9]);
        assert_eq!(filter.instrument_count(), 3);
    }

    #[test]
    fn filters_instruments() {
        let filter = SubscriptionFilter::for_instruments(&[1u32, 3], 8).unwrap();

        assert!(filter.matches(Some(&1), EventKind::Trade));
        assert!(!filter.matches(Some(&2), EventKind::Trade));
        assert!(filter.matches(Some(&3), EventKind::Trade));
    }

    #[test]
    fn filters_event_kinds() {
        let filter = SubscriptionFilter::<u32>::all().with_event_kinds(&[EventKind::Trade]);

        assert!(filter.matches(Some(&1), EventKind::Trade));
        assert!(!filter.matches(Some(&1), EventKind::Quote));
    }

    #[test]
    fn excludes_global_events() {
        let filter = SubscriptionFilter::<u32>::all().with_global_events(false);

        assert!(!filter.matches(None, EventKind::Heartbeat));
        assert!(filter.matches(Some(&1), EventKind::Trade));
    }

    #[test]
    fn rejects_oversized_instrument_list() {
        let result = SubscriptionFilter::for_instruments(&[1u32, 2, 3], 2);

        assert_eq!(
            result,
            Err(PublisherError::InstrumentLimitReached { limit: 2 })
        );
    }

    #[test]
    fn empty_instrument_list_is_not_wildcard() {
        let filter = SubscriptionFilter::<u32>::for_instruments(&[], 8).unwrap();

        assert!(!filter.includes_all_instruments());
        assert!(!filter.matches(Some(&1), EventKind::Trade));
    }
}
