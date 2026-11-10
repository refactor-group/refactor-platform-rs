use super::*;
use chrono::NaiveTime;

fn t(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).expect("valid test time")
}

fn user_json(extra: serde_json::Value) -> serde_json::Value {
    let mut base = serde_json::json!({
        "email": "hours@example.com",
        "first_name": "Wor",
        "last_name": "King",
    });
    let (Some(base_map), Some(extra_map)) = (base.as_object_mut(), extra.as_object()) else {
        panic!("test payloads are JSON objects");
    };
    base_map.extend(extra_map.clone());
    base
}

#[test]
fn defaults_are_eight_to_five_thirty() {
    assert_eq!(DEFAULT_START, t(8, 0));
    assert_eq!(DEFAULT_END, t(17, 30));
    assert_eq!(default_start(), DEFAULT_START);
    assert_eq!(default_end(), DEFAULT_END);
}

#[test]
fn parse_accepts_half_hour_steps() {
    for (input, expected) in [
        ("00:00", t(0, 0)),
        ("08:00", t(8, 0)),
        ("08:30", t(8, 30)),
        ("17:30", t(17, 30)),
        ("23:30", t(23, 30)),
    ] {
        assert_eq!(
            parse(Field::Start, input),
            Ok(expected),
            "parse should accept {input:?}"
        );
    }
}

#[test]
fn parse_rejects_anything_but_strict_hh_mm_on_the_step() {
    for input in [
        "8:00", "08:00:00", "08:15", "24:00", "23:45", "ab:cd", "", " 08:00", "08:00 ", "0800",
        "-1:00",
    ] {
        assert_eq!(
            parse(Field::End, input),
            Err(Invalid::NotOnStep(Field::End)),
            "parse should reject {input:?}"
        );
    }
}

#[test]
fn parse_names_the_field_it_was_given() {
    assert_eq!(
        parse(Field::Start, "08:15"),
        Err(Invalid::NotOnStep(Field::Start))
    );
}

#[test]
fn check_order_requires_start_before_end() {
    assert_eq!(check_order(t(8, 0), t(17, 30)), Ok(()));
    assert_eq!(check_order(t(0, 0), t(23, 30)), Ok(()));
    assert_eq!(
        check_order(t(8, 0), t(8, 0)),
        Err(Invalid::StartNotBeforeEnd)
    );
    assert_eq!(
        check_order(t(17, 30), t(8, 0)),
        Err(Invalid::StartNotBeforeEnd)
    );
}

#[test]
fn messages_match_the_agreed_wording() {
    assert_eq!(
        Invalid::NotOnStep(Field::Start).to_string(),
        "working_hours_start must be HH:MM on a 30-minute step"
    );
    assert_eq!(
        Invalid::NotOnStep(Field::End).to_string(),
        "working_hours_end must be HH:MM on a 30-minute step"
    );
    assert_eq!(
        Invalid::StartNotBeforeEnd.to_string(),
        "working_hours_start must be before working_hours_end"
    );
}

#[test]
fn user_without_working_hours_deserializes_to_defaults() {
    let user: crate::users::Model =
        serde_json::from_value(user_json(serde_json::json!({}))).expect("user deserializes");
    assert_eq!(user.working_hours_start, t(8, 0));
    assert_eq!(user.working_hours_end, t(17, 30));
}

#[test]
fn user_serializes_working_hours_as_hh_mm_strings() {
    let mut user: crate::users::Model =
        serde_json::from_value(user_json(serde_json::json!({}))).expect("user deserializes");
    user.working_hours_start = t(9, 30);
    user.working_hours_end = t(16, 0);
    let value = serde_json::to_value(&user).expect("user serializes");
    assert_eq!(value["working_hours_start"], serde_json::json!("09:30"));
    assert_eq!(value["working_hours_end"], serde_json::json!("16:00"));
}

#[test]
fn user_round_trips_explicit_working_hours() {
    let user: crate::users::Model = serde_json::from_value(user_json(serde_json::json!({
        "working_hours_start": "07:30",
        "working_hours_end": "15:00",
    })))
    .expect("user deserializes");
    assert_eq!(user.working_hours_start, t(7, 30));
    assert_eq!(user.working_hours_end, t(15, 0));
}

#[test]
fn user_with_off_step_working_hours_fails_to_deserialize() {
    let result: Result<crate::users::Model, _> = serde_json::from_value(user_json(
        serde_json::json!({ "working_hours_start": "08:15" }),
    ));
    assert!(result.is_err(), "08:15 must not deserialize");
}
