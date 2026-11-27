//! Deterministic replay cursor.
//!
//! The cursor advances only after the caller confirms
//! successful application of the expected event.

use maxwell_core::SequenceNumber;

use crate::error::{RecoveryError, RecoveryResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayCursor {
    next_expected: SequenceNumber,
}

impl ReplayCursor {
    #[must_use]
    pub const fn new(next_expected: SequenceNumber) -> Self {
        Self { next_expected }
    }

    #[must_use]
    pub const fn next_expected(self) -> SequenceNumber {
        self.next_expected
    }

    #[must_use]
    pub const fn last_applied(self) -> Option<SequenceNumber> {
        self.next_expected.previous()
    }

    pub fn validate(self, sequence: SequenceNumber) -> RecoveryResult<()> {
        if sequence != self.next_expected {
            return Err(RecoveryError::UnexpectedSequence {
                expected: self.next_expected,
                received: sequence,
            });
        }

        // Ensure advancement is possible before
        // allowing the external book to mutate.
        sequence.next().ok_or(RecoveryError::SequenceExhausted)?;

        Ok(())
    }

    pub fn commit(&mut self, sequence: SequenceNumber) -> RecoveryResult<()> {
        self.validate(sequence)?;

        self.next_expected = sequence.next().ok_or(RecoveryError::SequenceExhausted)?;

        Ok(())
    }

    pub fn reset(&mut self, next_expected: SequenceNumber) {
        self.next_expected = next_expected;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contiguous_replay() {
        let mut cursor = ReplayCursor::new(SequenceNumber::new(10));

        for sequence in 10..20 {
            cursor.commit(SequenceNumber::new(sequence)).unwrap();
        }

        assert_eq!(cursor.next_expected(), SequenceNumber::new(20),);
    }

    #[test]
    fn cannot_skip_missing_sequence() {
        let mut cursor = ReplayCursor::new(SequenceNumber::new(10));

        assert!(cursor.commit(SequenceNumber::new(11)).is_err());

        assert_eq!(cursor.next_expected(), SequenceNumber::new(10),);
    }

    #[test]
    fn sequence_exhaustion_is_explicit() {
        let mut cursor = ReplayCursor::new(SequenceNumber::MAX);

        assert_eq!(
            cursor.commit(SequenceNumber::MAX),
            Err(RecoveryError::SequenceExhausted),
        );
    }
}
