use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;

use chrono::{NaiveTime, Utc};
use entity::users::Model;
use entity::working_hours::{Field, Invalid};
use entity::Id;
use sea_orm::sqlx::error::{DatabaseError, ErrorKind};
use sea_orm::{DbErr, RuntimeErr, Value};

use super::{
    parse_working_hours, validate_working_hours_in_update_map, violated_working_hours_check,
    WorkingHoursPatch,
};
use crate::error::EntityApiErrorKind;
use crate::mutate::UpdateMap;

fn time(hour: u32, minute: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(hour, minute, 0).expect("valid test time")
}

/// A stored user working 08:00 to 17:30.
fn stored_user() -> Model {
    let now = Utc::now();
    Model {
        id: Id::new_v4(),
        email: "stored@example.com".to_string(),
        first_name: "Stored".to_string(),
        last_name: "User".to_string(),
        display_name: None,
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: 60,
        working_hours_start: time(8, 0),
        working_hours_end: time(17, 30),
        roles: vec![],
        invite_status: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn update_map(entries: &[(&str, Value)]) -> UpdateMap {
    entries
        .iter()
        .fold(UpdateMap::new(), |mut map, (key, value)| {
            map.insert(key.to_string(), Some(value.clone()));
            map
        })
}

fn chrono_time(hour: u32, minute: u32) -> Value {
    Value::ChronoTime(Some(time(hour, minute)))
}

fn working_hours_error(result: Result<(), crate::error::Error>) -> Option<Invalid> {
    match result {
        Err(err) => match err.error_kind {
            EntityApiErrorKind::WorkingHours(invalid) => Some(invalid),
            _ => None,
        },
        Ok(()) => None,
    }
}

#[test]
fn parse_working_hours_with_neither_field_changes_nothing() {
    assert_eq!(
        parse_working_hours(None, None),
        Ok(WorkingHoursPatch {
            start: None,
            end: None
        })
    );
}

#[test]
fn parse_working_hours_with_only_a_start_parses_the_start() {
    assert_eq!(
        parse_working_hours(Some("09:00"), None),
        Ok(WorkingHoursPatch {
            start: Some(time(9, 0)),
            end: None
        })
    );
}

#[test]
fn parse_working_hours_rejects_an_end_off_the_format() {
    let err = parse_working_hours(Some("09:00"), Some("9:30")).expect_err("9:30 is not HH:MM");
    assert_eq!(
        err.error_kind,
        EntityApiErrorKind::WorkingHours(Invalid::NotOnStep(Field::End))
    );
}

#[test]
fn parse_working_hours_reports_the_start_first() {
    let err = parse_working_hours(Some("08:15"), Some("9:30")).expect_err("both are invalid");
    assert_eq!(
        err.error_kind,
        EntityApiErrorKind::WorkingHours(Invalid::NotOnStep(Field::Start))
    );
}

#[test]
fn validate_working_hours_rejects_a_start_after_the_stored_end() {
    let map = update_map(&[("working_hours_start", chrono_time(18, 0))]);
    assert_eq!(
        working_hours_error(validate_working_hours_in_update_map(&map, &stored_user())),
        Some(Invalid::StartNotBeforeEnd)
    );
}

#[test]
fn validate_working_hours_rejects_an_end_before_the_stored_start() {
    let map = update_map(&[("working_hours_end", chrono_time(7, 0))]);
    assert_eq!(
        working_hours_error(validate_working_hours_in_update_map(&map, &stored_user())),
        Some(Invalid::StartNotBeforeEnd)
    );
}

#[test]
fn validate_working_hours_checks_both_sent_values_against_each_other() {
    let map = update_map(&[
        ("working_hours_start", chrono_time(18, 0)),
        ("working_hours_end", chrono_time(20, 0)),
    ]);
    assert_eq!(
        validate_working_hours_in_update_map(&map, &stored_user()),
        Ok(())
    );
}

#[test]
fn validate_working_hours_accepts_a_start_before_the_stored_end() {
    let map = update_map(&[("working_hours_start", chrono_time(9, 30))]);
    assert_eq!(
        validate_working_hours_in_update_map(&map, &stored_user()),
        Ok(())
    );
}

#[test]
fn validate_working_hours_passes_a_map_without_working_hours() {
    let map = update_map(&[("first_name", Value::String(Some("Changed".to_string())))]);
    assert_eq!(
        validate_working_hours_in_update_map(&map, &stored_user()),
        Ok(())
    );
}

#[test]
fn validate_working_hours_rejects_a_start_that_is_not_a_time() {
    let map = update_map(&[("working_hours_start", Value::String(Some("18:00".into())))]);
    assert_eq!(
        working_hours_error(validate_working_hours_in_update_map(&map, &stored_user())),
        Some(Invalid::NotOnStep(Field::Start))
    );
}

/// A database error naming a constraint, standing in for Postgres.
#[derive(Debug)]
struct ConstraintError(&'static str);

impl fmt::Display for ConstraintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "violates check constraint {}", self.0)
    }
}

impl StdError for ConstraintError {}

impl DatabaseError for ConstraintError {
    fn message(&self) -> &str {
        "check constraint violated"
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed("23514"))
    }

    fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
        self
    }

    fn constraint(&self) -> Option<&str> {
        Some(self.0)
    }

    fn kind(&self) -> ErrorKind {
        ErrorKind::CheckViolation
    }
}

fn check_violation(constraint: &'static str) -> DbErr {
    DbErr::Query(RuntimeErr::SqlxError(Arc::new(
        sea_orm::sqlx::Error::Database(Box::new(ConstraintError(constraint))),
    )))
}

#[test]
fn working_hours_check_violations_map_to_the_broken_rule() {
    assert_eq!(
        violated_working_hours_check(&check_violation("users_working_hours_order")),
        Some(Invalid::StartNotBeforeEnd)
    );
    assert_eq!(
        violated_working_hours_check(&check_violation("users_working_hours_start_step")),
        Some(Invalid::NotOnStep(Field::Start))
    );
    assert_eq!(
        violated_working_hours_check(&check_violation("users_working_hours_end_step")),
        Some(Invalid::NotOnStep(Field::End))
    );
}

#[test]
fn other_database_errors_are_not_working_hours_violations() {
    assert_eq!(
        violated_working_hours_check(&check_violation("some_other_check")),
        None
    );
    assert_eq!(violated_working_hours_check(&DbErr::RecordNotUpdated), None);
}
