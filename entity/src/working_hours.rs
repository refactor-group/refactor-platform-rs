//! A user's working hours: local wall-clock times in the user's `timezone`.
//!
//! A non-table value module like `entity::duration`. Times travel as strict
//! `"HH:MM"` on a 30-minute step, and the start must come before the end.

use std::fmt;

use chrono::NaiveTime;

const STEP_MINUTES: u32 = 30;

pub const DEFAULT_START: NaiveTime = hm(8, 0);
pub const DEFAULT_END: NaiveTime = hm(17, 30);

const fn hm(hour: u32, minute: u32) -> NaiveTime {
    match NaiveTime::from_hms_opt(hour, minute, 0) {
        Some(time) => time,
        None => panic!("invalid working hours constant"),
    }
}

/// Default start time, for serde defaults and struct literals.
pub fn default_start() -> NaiveTime {
    DEFAULT_START
}

/// Default end time, for serde defaults and struct literals.
pub fn default_end() -> NaiveTime {
    DEFAULT_END
}

/// Which working-hours field a value belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Start,
    End,
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Field::Start => "working_hours_start",
            Field::End => "working_hours_end",
        })
    }
}

/// Why a working-hours value was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    NotOnStep(Field),
    StartNotBeforeEnd,
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Invalid::NotOnStep(field) => {
                write!(f, "{field} must be HH:MM on a 30-minute step")
            }
            Invalid::StartNotBeforeEnd => {
                f.write_str("working_hours_start must be before working_hours_end")
            }
        }
    }
}

impl std::error::Error for Invalid {}

/// Strict "HH:MM" on a 30-minute step. `field` names the field in the error message.
pub fn parse(field: Field, value: &str) -> Result<NaiveTime, Invalid> {
    let invalid = || Invalid::NotOnStep(field);
    let [h1, h2, b':', m1, m2] = value.as_bytes() else {
        return Err(invalid());
    };
    let two_digits = |tens: u8, ones: u8| {
        (tens.is_ascii_digit() && ones.is_ascii_digit())
            .then(|| u32::from(tens - b'0') * 10 + u32::from(ones - b'0'))
    };
    let hour = two_digits(*h1, *h2)
        .filter(|h| *h < 24)
        .ok_or_else(invalid)?;
    let minute = two_digits(*m1, *m2)
        .filter(|m| m % STEP_MINUTES == 0)
        .ok_or_else(invalid)?;
    NaiveTime::from_hms_opt(hour, minute, 0).ok_or_else(invalid)
}

/// start < end, both already on the step.
pub fn check_order(start: NaiveTime, end: NaiveTime) -> Result<(), Invalid> {
    (start < end)
        .then_some(())
        .ok_or(Invalid::StartNotBeforeEnd)
}

/// Serde module for a NaiveTime field as exactly "HH:MM".
pub mod hh_mm {
    use chrono::NaiveTime;
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(time: &NaiveTime, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&time.format("%H:%M"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NaiveTime, D::Error> {
        let value = String::deserialize(deserializer)?;
        // The field is unknown here, so the message stays neutral.
        super::parse(super::Field::Start, &value)
            .map_err(|_| de::Error::custom("expected HH:MM on a 30-minute step"))
    }
}

#[cfg(test)]
#[path = "working_hours_tests.rs"]
mod tests;
