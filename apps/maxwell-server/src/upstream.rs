//! Upstream market data decoding and feed validation.
//!
//! This module defines the boundary between external wire
//! representations and canonical Maxwell market events.
//!
//! The initial adapter supports the native Maxwell binary
//! protocol. Planck interoperability will be implemented
//! separately after its wire contract has been verified.
//!
//! Transport framing must be removed before decoding.

use maxwell_core::{EventEnvelope, FeedId};
use maxwell_protocol::Decoder;

use crate::error::{ServerError, ServerResult};

// ============================================================
// Upstream decoder
// ============================================================

/// Converts complete upstream message payloads into
/// canonical Maxwell market data events.
///
/// Implementations must preserve authoritative feed
/// identity and sequence semantics.
///
/// Implementations must not mutate reconstructed books
/// or advance application sequence cursors.
pub trait UpstreamDecoder {
    /// Decodes exactly one complete upstream message.
    fn decode(&self, payload: &[u8]) -> ServerResult<EventEnvelope>;
}

// ============================================================
// Native Maxwell decoder
// ============================================================

/// Adapter for the native Maxwell binary protocol.
///
/// The underlying protocol decoder validates message
/// headers, payload layouts, versions, and event fields.
pub struct MaxwellDecoder {
    decoder: Decoder,
}

impl MaxwellDecoder {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            decoder: Decoder::new(),
        }
    }
}

impl Default for MaxwellDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl UpstreamDecoder for MaxwellDecoder {
    fn decode(&self, payload: &[u8]) -> ServerResult<EventEnvelope> {
        Ok(self.decoder.decode_event(payload)?)
    }
}

// ============================================================
// Upstream feed
// ============================================================

/// Feed-scoped upstream decoding adapter.
///
/// Feed identity is established by the application during
/// initialization or authoritative session establishment.
///
/// An unexpected feed identity is rejected rather than
/// silently interpreted as a new session.
pub struct Upstream<D = MaxwellDecoder> {
    feed: FeedId,
    decoder: D,
}

impl Upstream<MaxwellDecoder> {
    /// Constructs an upstream adapter for the native
    /// Maxwell protocol.
    #[must_use]
    pub const fn native(feed: FeedId) -> Self {
        Self::new(feed, MaxwellDecoder::new())
    }
}

impl<D> Upstream<D> {
    /// Constructs an upstream adapter using the supplied
    /// wire protocol decoder.
    #[must_use]
    pub const fn new(feed: FeedId, decoder: D) -> Self {
        Self { feed, decoder }
    }

    /// Returns the authoritative feed identity.
    #[must_use]
    pub const fn feed(&self) -> FeedId {
        self.feed
    }

    /// Returns the configured decoder.
    #[must_use]
    pub const fn decoder(&self) -> &D {
        &self.decoder
    }

    /// Updates the expected feed identity.
    ///
    /// This operation must be coordinated with recovery
    /// state, book installation, and subscriber invalidation.
    ///
    /// It does not establish synchronization by itself.
    pub fn replace_feed(&mut self, feed: FeedId) {
        self.feed = feed;
    }

    /// Validates a canonical event against the configured
    /// feed and stateless event invariants.
    ///
    /// Sequence continuity is deliberately not validated
    /// here. That responsibility belongs to the recovery
    /// coordinator.
    pub fn validate_event(&self, event: &EventEnvelope) -> ServerResult<()> {
        if event.header.feed() != self.feed {
            return Err(ServerError::InvalidState(
                "unexpected upstream feed or session",
            ));
        }

        event
            .event
            .validate()
            .map_err(|_| ServerError::InvalidState("invalid upstream market event"))?;

        Ok(())
    }
}

impl<D: UpstreamDecoder> Upstream<D> {
    /// Decodes and validates a complete upstream payload.
    ///
    /// TCP length prefixes and other transport-specific
    /// framing must already have been removed.
    ///
    /// This operation does not change processing state.
    pub fn decode(&self, payload: &[u8]) -> ServerResult<EventEnvelope> {
        let event = self.decoder.decode(payload)?;

        self.validate_event(&event)?;

        Ok(event)
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    use maxwell_core::{
        EventHeader, EventId, InstrumentId, MarketEvent, OrderDeleted, OrderId, SequenceNumber,
        SessionId, StreamId,
    };
    use maxwell_protocol::Encoder;

    fn feed() -> FeedId {
        FeedId::new(StreamId::new(1), SessionId::new(1))
    }

    fn event() -> EventEnvelope {
        EventEnvelope::new(
            EventHeader::new(
                EventId::new(feed(), SequenceNumber::ONE),
                InstrumentId::new(1),
                None,
            ),
            MarketEvent::OrderDeleted(OrderDeleted {
                order_id: OrderId::new(100),
            }),
        )
    }

    // --------------------------------------------------------
    // Native protocol
    // --------------------------------------------------------

    #[test]
    fn native_decoder_rejects_empty_payload() {
        let decoder = MaxwellDecoder::new();

        assert!(matches!(decoder.decode(&[]), Err(ServerError::Protocol(_)),));
    }

    #[test]
    fn native_decoder_rejects_truncated_payload() {
        let decoder = MaxwellDecoder::new();

        assert!(matches!(
            decoder.decode(&[0; 4]),
            Err(ServerError::Protocol(_)),
        ));
    }

    #[test]
    fn native_protocol_round_trip() {
        let upstream = Upstream::native(feed());
        let original = event();

        let mut buffer = [0_u8; 1024];

        let written = Encoder::new().encode_event(original, &mut buffer).unwrap();

        let decoded = upstream.decode(&buffer[..written]).unwrap();

        assert_eq!(decoded, original);
    }

    #[test]
    fn native_decoder_rejects_trailing_bytes() {
        let upstream = Upstream::native(feed());

        let mut buffer = [0_u8; 1024];

        let written = Encoder::new().encode_event(event(), &mut buffer).unwrap();

        assert!(matches!(
            upstream.decode(&buffer[..written + 1]),
            Err(ServerError::Protocol(_)),
        ));
    }

    // --------------------------------------------------------
    // Feed validation
    // --------------------------------------------------------

    #[test]
    fn accepts_configured_feed() {
        let upstream = Upstream::native(feed());

        assert!(upstream.validate_event(&event()).is_ok());
    }

    #[test]
    fn rejects_unexpected_session() {
        let upstream = Upstream::native(feed());

        let mut unexpected = event();

        unexpected.header.id.feed.session_id = SessionId::new(2);

        assert!(matches!(
            upstream.validate_event(&unexpected),
            Err(ServerError::InvalidState(_)),
        ));
    }

    #[test]
    fn rejects_unexpected_stream() {
        let upstream = Upstream::native(feed());

        let mut unexpected = event();

        unexpected.header.id.feed.stream_id = StreamId::new(2);

        assert!(upstream.validate_event(&unexpected).is_err());
    }

    #[test]
    fn explicit_feed_replacement_changes_validation() {
        let mut upstream = Upstream::native(feed());

        let replacement = FeedId::new(StreamId::new(1), SessionId::new(2));

        upstream.replace_feed(replacement);

        assert_eq!(upstream.feed(), replacement);
        assert!(upstream.validate_event(&event()).is_err());
    }

    // --------------------------------------------------------
    // Decoder abstraction
    // --------------------------------------------------------

    struct SyntheticDecoder {
        event: EventEnvelope,
    }

    impl UpstreamDecoder for SyntheticDecoder {
        fn decode(&self, _payload: &[u8]) -> ServerResult<EventEnvelope> {
            Ok(self.event)
        }
    }

    #[test]
    fn supports_alternative_wire_adapters() {
        let original = event();

        let upstream = Upstream::new(feed(), SyntheticDecoder { event: original });

        assert_eq!(upstream.decode(&[]).unwrap(), original,);
    }

    #[test]
    fn alternative_decoder_cannot_bypass_feed_validation() {
        let mut unexpected = event();

        unexpected.header.id.feed.session_id = SessionId::new(99);

        let upstream = Upstream::new(feed(), SyntheticDecoder { event: unexpected });

        assert!(upstream.decode(&[]).is_err());
    }
}
