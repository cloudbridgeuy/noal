//! Time as a value.
//!
//! The core never asks what time it is. The shell reads the clock once at the
//! edge of a request and passes a `Timestamp` inward. That keeps every rule
//! that depends on time deterministic, and therefore testable.

use serde::{Deserialize, Serialize};

/// A point in time, as whole seconds since the Unix epoch.
///
/// This is a newtype rather than a bare `i64` so a timestamp cannot be
/// confused with a duration, an identifier, or a count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(i64);

impl Timestamp {
    /// Build a timestamp from whole seconds since the Unix epoch.
    #[must_use]
    pub const fn from_unix_seconds(seconds: i64) -> Self {
        Self(seconds)
    }

    /// The seconds since the Unix epoch.
    #[must_use]
    pub const fn as_unix_seconds(self) -> i64 {
        self.0
    }

    /// Move forward by a whole number of seconds.
    ///
    /// Saturates instead of overflowing, because a session deadline far in the
    /// future is safe and a panic is not.
    #[must_use]
    pub const fn plus_seconds(self, seconds: i64) -> Self {
        Self(self.0.saturating_add(seconds))
    }

    /// True when `self` is strictly later than `other`.
    #[must_use]
    pub const fn is_after(self, other: Self) -> bool {
        self.0 > other.0
    }
}

#[cfg(test)]
mod tests {
    use super::Timestamp;

    #[test]
    fn round_trips_through_unix_seconds() {
        let stamp = Timestamp::from_unix_seconds(1_700_000_000);
        assert_eq!(stamp.as_unix_seconds(), 1_700_000_000);
    }

    #[test]
    fn plus_seconds_moves_forward() {
        let stamp = Timestamp::from_unix_seconds(100);
        assert_eq!(stamp.plus_seconds(50), Timestamp::from_unix_seconds(150));
    }

    #[test]
    fn plus_seconds_saturates_instead_of_overflowing() {
        let stamp = Timestamp::from_unix_seconds(i64::MAX);
        assert_eq!(
            stamp.plus_seconds(1),
            Timestamp::from_unix_seconds(i64::MAX)
        );
    }

    #[test]
    fn is_after_is_strict() {
        let early = Timestamp::from_unix_seconds(10);
        let late = Timestamp::from_unix_seconds(20);
        assert!(late.is_after(early));
        assert!(!early.is_after(late));
        assert!(!early.is_after(early));
    }

    #[test]
    fn ordering_follows_the_instant() {
        assert!(Timestamp::from_unix_seconds(1) < Timestamp::from_unix_seconds(2));
    }
}
