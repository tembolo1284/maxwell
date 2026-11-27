//! Protocol version representation and compatibility rules.

use core::fmt;

use crate::constants::{VERSION_MAJOR, VERSION_MINOR};
use crate::error::{ProtocolError, ProtocolResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProtocolVersion {
    pub major: u8,
    pub minor: u8,
}

impl ProtocolVersion {
    pub const V1: Self = Self::new(1, 0);

    pub const CURRENT: Self = Self::new(VERSION_MAJOR, VERSION_MINOR);

    #[must_use]
    #[inline]
    pub const fn new(major: u8, minor: u8) -> Self {
        Self { major, minor }
    }

    /// Only explicitly implemented versions are supported.
    #[must_use]
    #[inline]
    pub const fn is_supported(self) -> bool {
        matches!((self.major, self.minor), (1, 0))
    }

    /// Validates that this version is implemented.
    ///
    /// # Errors
    ///
    /// Returns UnsupportedVersion for unknown versions.
    #[inline]
    pub const fn validate(self) -> ProtocolResult<()> {
        if self.is_supported() {
            Ok(())
        } else {
            Err(ProtocolError::UnsupportedVersion {
                major: self.major,
                minor: self.minor,
            })
        }
    }
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        Self::CURRENT
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_supported() {
        assert!(ProtocolVersion::CURRENT.is_supported());
    }

    #[test]
    fn unknown_major_rejected() {
        assert_eq!(
            ProtocolVersion::new(2, 0).validate(),
            Err(ProtocolError::UnsupportedVersion { major: 2, minor: 0 })
        );
    }

    #[test]
    fn unknown_minor_rejected() {
        assert!(!ProtocolVersion::new(1, 1).is_supported());
    }

    #[test]
    fn default_version() {
        assert_eq!(ProtocolVersion::default(), ProtocolVersion::CURRENT);
    }

    #[test]
    fn version_display() {
        assert_eq!(ProtocolVersion::V1.to_string(), "1.0");
    }
}
