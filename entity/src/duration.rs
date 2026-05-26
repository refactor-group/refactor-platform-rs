//! Validated coaching-session duration as a newtype around `i16`.
//!
//! The `Duration` newtype wraps a primitive minute count and carries three
//! concerns: the validation rule (`1..=480`), the storage type bound (matches
//! the PG `SMALLINT` column on `coaching_sessions`, which sqlx-postgres
//! decodes as `i16`), and the human-readable formatting via `Display`.
//!
//! `Duration` follows the same pattern as `entity::provider::Provider`: a
//! non-table value type that constrains the set of values for a DB column.
//!
//! The inner type is `i16` rather than `u16` because PostgreSQL's `SMALLINT`
//! is signed and sqlx-postgres has no `u16` codec. Using `i16` throughout
//! eliminates any `u16 ↔ i16` conversions at storage boundaries. The
//! validation range `1..=480` enforces non-negative values at runtime.

use std::fmt;

pub const MIN_DURATION_MINUTES: i16 = 1;
pub const MAX_DURATION_MINUTES: i16 = 480;
const DEFAULT_DURATION_MINUTES: i16 = 60;

/// A validated coaching-session duration in minutes, guaranteed to be within
/// `MIN_DURATION_MINUTES..=MAX_DURATION_MINUTES`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Duration(i16);

impl Duration {
    /// Construct a `Duration`, validating the range.
    pub fn new(minutes: i16) -> Result<Self, OutOfRange> {
        if (MIN_DURATION_MINUTES..=MAX_DURATION_MINUTES).contains(&minutes) {
            Ok(Self(minutes))
        } else {
            Err(OutOfRange {
                got: minutes,
                min: MIN_DURATION_MINUTES,
                max: MAX_DURATION_MINUTES,
            })
        }
    }

    /// Construct a `Duration` without checking the range.
    ///
    /// This is the escape hatch for values that are already known to be valid
    /// by an external invariant — most importantly DB rows where the column is
    /// `NOT NULL` and the only writes go through `Duration::new`. Do not use on
    /// untrusted input.
    pub const fn from_minutes_unchecked(minutes: i16) -> Self {
        Self(minutes)
    }

    /// The wrapped minute count.
    pub const fn minutes(self) -> i16 {
        self.0
    }

    /// The application's default duration in minutes.
    ///
    /// Use this when constructing entity Model literals or seeding column
    /// defaults. For a `Duration` instance, use `Duration::default()`.
    pub const fn default_minutes() -> i16 {
        DEFAULT_DURATION_MINUTES
    }
}

impl TryFrom<i16> for Duration {
    type Error = OutOfRange;

    fn try_from(value: i16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Default for Duration {
    /// Returns the application's default `Duration` (60 minutes).
    fn default() -> Self {
        Self(DEFAULT_DURATION_MINUTES)
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.0;
        let hours = total / 60;
        let mins = total % 60;

        match (hours, mins) {
            (0, 0) => unreachable!("Duration invariant violated: value must be >= 1 minute"),
            (0, 1) => write!(f, "1 minute"),
            (0, m) => write!(f, "{m} minutes"),
            (1, 0) => write!(f, "1 hour"),
            (h, 0) => write!(f, "{h} hours"),
            (1, 1) => write!(f, "1 hour 1 minute"),
            (1, m) => write!(f, "1 hour {m} minutes"),
            (h, 1) => write!(f, "{h} hours 1 minute"),
            (h, m) => write!(f, "{h} hours {m} minutes"),
        }
    }
}

/// Error returned when constructing a `Duration` with an out-of-range value.
///
/// Lives in the entity layer alongside the type it constrains. Higher layers
/// (entity_api, etc.) map this to their own error kinds at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct OutOfRange {
    pub got: i16,
    pub min: i16,
    pub max: i16,
}

impl fmt::Display for OutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "duration_minutes must be between {} and {} (got {})",
            self.min, self.max, self.got
        )
    }
}

impl std::error::Error for OutOfRange {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_accepts_min() {
        assert_eq!(Duration::new(1).unwrap().minutes(), 1);
    }

    #[test]
    fn new_accepts_default() {
        assert_eq!(Duration::new(60).unwrap().minutes(), 60);
    }

    #[test]
    fn new_accepts_max() {
        assert_eq!(Duration::new(480).unwrap().minutes(), 480);
    }

    #[test]
    fn new_rejects_zero() {
        let err = Duration::new(0).unwrap_err();
        assert_eq!(err.got, 0);
        assert_eq!(err.min, 1);
        assert_eq!(err.max, 480);
    }

    #[test]
    fn new_rejects_negative() {
        assert!(Duration::new(-1).is_err());
    }

    #[test]
    fn new_rejects_above_max() {
        assert!(Duration::new(481).is_err());
    }

    #[test]
    fn new_rejects_i16_max() {
        assert!(Duration::new(i16::MAX).is_err());
    }

    #[test]
    fn try_from_mirrors_new() {
        assert!(Duration::try_from(60_i16).is_ok());
        assert!(Duration::try_from(0_i16).is_err());
        assert!(Duration::try_from(481_i16).is_err());
        assert!(Duration::try_from(-1_i16).is_err());
    }

    #[test]
    fn display_one_minute() {
        assert_eq!(Duration::new(1).unwrap().to_string(), "1 minute");
    }

    #[test]
    fn display_multi_minutes() {
        assert_eq!(Duration::new(45).unwrap().to_string(), "45 minutes");
    }

    #[test]
    fn display_one_hour_exact() {
        assert_eq!(Duration::new(60).unwrap().to_string(), "1 hour");
    }

    #[test]
    fn display_one_hour_one_minute() {
        assert_eq!(Duration::new(61).unwrap().to_string(), "1 hour 1 minute");
    }

    #[test]
    fn display_one_hour_multi_minutes() {
        assert_eq!(Duration::new(62).unwrap().to_string(), "1 hour 2 minutes");
    }

    #[test]
    fn display_one_hour_thirty() {
        assert_eq!(Duration::new(90).unwrap().to_string(), "1 hour 30 minutes");
    }

    #[test]
    fn display_multi_hours_exact() {
        assert_eq!(Duration::new(120).unwrap().to_string(), "2 hours");
    }

    #[test]
    fn display_multi_hours_one_minute() {
        assert_eq!(Duration::new(121).unwrap().to_string(), "2 hours 1 minute");
    }

    #[test]
    fn display_multi_hours_multi_minutes() {
        assert_eq!(Duration::new(122).unwrap().to_string(), "2 hours 2 minutes");
    }

    #[test]
    fn display_max() {
        assert_eq!(Duration::new(480).unwrap().to_string(), "8 hours");
    }

    #[test]
    fn out_of_range_display_includes_value() {
        let err = Duration::new(481).unwrap_err();
        assert!(err.to_string().contains("481"));
        assert!(err.to_string().contains("1"));
        assert!(err.to_string().contains("480"));
    }

    #[test]
    fn from_minutes_unchecked_preserves_value() {
        let d = Duration::from_minutes_unchecked(45);
        assert_eq!(d.minutes(), 45);
    }

    #[test]
    fn default_returns_60() {
        assert_eq!(Duration::default().minutes(), 60);
    }

    #[test]
    fn default_minutes_returns_60() {
        assert_eq!(Duration::default_minutes(), 60);
    }
}
