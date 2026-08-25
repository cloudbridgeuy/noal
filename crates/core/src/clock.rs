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

    /// The stored instant as an RFC 3339 timestamp in UTC, second precision.
    ///
    /// Pure formatting of the value this newtype already holds; reading no
    /// clock keeps `state::now()` the only place noal asks what time it is.
    #[must_use]
    pub fn to_rfc3339(self) -> String {
        let instant = chrono::DateTime::from_timestamp(self.0, 0)
            .unwrap_or(chrono::DateTime::UNIX_EPOCH);
        instant.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    /// The stored date alone, UTC, for a reader deciding whether a window is
    /// old or recent. Day granularity: noal holds no viewer timezone, and the
    /// full instant stays available through [`Self::to_rfc3339`].
    #[must_use]
    pub fn display_date(self) -> String {
        let instant = chrono::DateTime::from_timestamp(self.0, 0)
            .unwrap_or(chrono::DateTime::UNIX_EPOCH);
        instant.format("%-d %b %Y").to_string()
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

    #[test]
    fn display_date_is_day_granular_utc_without_leading_zero() {
        // 2026-08-21T14:03:09Z — day only, month abbreviated, no zero padding.
        let stamp = Timestamp::from_unix_seconds(1_787_320_989);
        assert_eq!(stamp.display_date(), "21 Aug 2026");
    }

    #[test]
    fn display_date_handles_the_first_of_a_month() {
        let stamp = Timestamp::from_unix_seconds(1_746_057_600); // 2025-05-01T00:00Z
        assert_eq!(stamp.display_date(), "1 May 2025");
    }

    #[test]
    fn rfc3339_carries_the_full_instant_in_utc() {
        let stamp = Timestamp::from_unix_seconds(1_787_320_989);
        assert_eq!(stamp.to_rfc3339(), "2026-08-21T14:03:09Z");
    }
}
