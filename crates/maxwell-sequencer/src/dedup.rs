//! Stateless sequence classification.
//!
//! Previously delivered sequence numbers are classified
//! using the next expected sequence watermark.
//!
//! Buffered duplicates are detected by the reorder buffer.
//! No unbounded historical hash set is required.

use maxwell_core::SequenceNumber;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceDisposition {
    Expected,
    PreviouslyDelivered,
    Future { distance: u64 },
    Exhausted,
}

impl SequenceDisposition {
    #[must_use]
    #[inline]
    pub const fn classify(expected: Option<SequenceNumber>, received: SequenceNumber) -> Self {
        let Some(expected) = expected else {
            return Self::Exhausted;
        };

        let expected = expected.raw();
        let received = received.raw();

        if received == expected {
            Self::Expected
        } else if received < expected {
            Self::PreviouslyDelivered
        } else {
            Self::Future {
                distance: received - expected,
            }
        }
    }

    #[must_use]
    #[inline]
    pub const fn is_duplicate(self) -> bool {
        matches!(self, Self::PreviouslyDelivered)
    }

    #[must_use]
    #[inline]
    pub const fn is_expected(self) -> bool {
        matches!(self, Self::Expected)
    }

    #[must_use]
    #[inline]
    pub const fn is_future(self) -> bool {
        matches!(self, Self::Future { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(value: u64) -> SequenceNumber {
        SequenceNumber::new(value)
    }

    #[test]
    fn expected_sequence() {
        assert_eq!(
            SequenceDisposition::classify(Some(seq(100)), seq(100),),
            SequenceDisposition::Expected,
        );
    }

    #[test]
    fn previously_delivered_sequence() {
        assert_eq!(
            SequenceDisposition::classify(Some(seq(100)), seq(99),),
            SequenceDisposition::PreviouslyDelivered,
        );
    }

    #[test]
    fn future_sequence() {
        assert_eq!(
            SequenceDisposition::classify(Some(seq(100)), seq(105),),
            SequenceDisposition::Future { distance: 5 },
        );
    }

    #[test]
    fn exhausted_sequence() {
        assert_eq!(
            SequenceDisposition::classify(None, seq(100)),
            SequenceDisposition::Exhausted,
        );
    }

    #[test]
    fn maximum_distance() {
        assert_eq!(
            SequenceDisposition::classify(Some(seq(0)), seq(u64::MAX),),
            SequenceDisposition::Future { distance: u64::MAX },
        );
    }
}
