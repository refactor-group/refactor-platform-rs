//! How a "this and following" reschedule pairs sessions with slots and decides to split.
//! Pure: no database.

use chrono::{NaiveDate, TimeDelta};

use super::*;

/// Monday 2026-11-09 15:00 UTC, the old series' first slot.
fn first_slot() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 11, 9)
        .unwrap()
        .and_hms_opt(15, 0, 0)
        .unwrap()
}

fn week(n: i64) -> NaiveDateTime {
    first_slot() + TimeDelta::weeks(n)
}

fn now() -> NaiveDateTime {
    first_slot() - TimeDelta::days(3)
}

fn weekly(count: u32) -> Recurrence {
    Recurrence {
        frequency: Frequency::Weekly,
        interval: 1,
        by_weekdays: None,
        count: Some(count),
        until: None,
    }
}

fn existing_rule(recurrence: Recurrence) -> SeriesRule {
    SeriesRule {
        start_at: first_slot(),
        recurrence,
        duration_minutes: 60,
        timezone: "UTC".to_string(),
    }
}

fn session(slot: NaiveDateTime) -> coaching_sessions::Model {
    let now = chrono::Utc::now();
    coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id: Id::nil(),
        coaching_session_series_id: Some(Id::nil()),
        ical_sequence: 0,
        ical_recurrence_id: Some(slot),
        collab_document_name: None,
        date: slot,
        duration_minutes: 60,
        title: None,
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: None,
        notice_given_at: now.into(),
    }
}

/// One session per old slot, weeks `0..n`.
fn sessions(n: i64) -> Vec<coaching_sessions::Model> {
    (0..n).map(|i| session(week(i))).collect()
}

fn request(
    anchor: &coaching_sessions::Model,
    start_at: NaiveDateTime,
    recurrence: Option<Recurrence>,
) -> FollowingReschedule {
    FollowingReschedule {
        coaching_session_id: anchor.id,
        start_at,
        duration: None,
        recurrence,
    }
}

fn plan(
    rule: &SeriesRule,
    rows: &[coaching_sessions::Model],
    request: &FollowingReschedule,
) -> Result<FollowingPlan, Error> {
    plan_following(rule, rows, request, chrono_tz::UTC, now())
}

fn ids(sessions: &[coaching_sessions::Model]) -> Vec<Id> {
    sessions.iter().map(|session| session.id).collect()
}

fn moved_ids(plan: &FollowingPlan) -> Vec<Id> {
    plan.moves.iter().map(|(session, _)| session.id).collect()
}

fn is_validation(err: &Error) -> bool {
    matches!(err.error_kind, DomainErrorKind::Validation(_))
}

#[test]
fn the_kth_affected_session_moves_to_the_kth_slot_and_extra_slots_are_created() {
    let rows = sessions(4);
    let tuesday = week(1) + TimeDelta::days(1);
    let plan = plan(
        &existing_rule(weekly(4)),
        &rows,
        &request(&rows[1], tuesday, Some(weekly(5))),
    )
    .unwrap();

    assert_eq!(moved_ids(&plan), ids(&rows[1..]));
    assert_eq!(
        plan.moves.iter().map(|(_, slot)| *slot).collect::<Vec<_>>(),
        (0..3)
            .map(|i| tuesday + TimeDelta::weeks(i))
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        plan.extras,
        vec![tuesday + TimeDelta::weeks(3), tuesday + TimeDelta::weeks(4)]
    );
    assert!(plan.leftovers.is_empty());
}

#[test]
fn fewer_slots_leave_the_last_affected_sessions_over() {
    let rows = sessions(5);
    let plan = plan(
        &existing_rule(weekly(5)),
        &rows,
        &request(&rows[1], week(1), Some(weekly(2))),
    )
    .unwrap();

    assert_eq!(moved_ids(&plan), ids(&rows[1..3]));
    assert!(plan.extras.is_empty());
    assert_eq!(ids(&plan.leftovers), ids(&rows[3..]));
    assert_eq!(plan.leftover_ids(), ids(&rows[3..]));
}

#[test]
fn affected_ids_are_the_moves_then_the_leftovers_in_date_order() {
    let rows = sessions(4);
    let plan = plan(
        &existing_rule(weekly(4)),
        &rows,
        &request(&rows[0], week(0), Some(weekly(2))),
    )
    .unwrap();

    assert_eq!(plan.affected_ids(), ids(&rows));
}

#[test]
fn an_omitted_count_rule_keeps_the_number_of_affected_sessions() {
    let rows = sessions(6);
    let tuesday = week(2) + TimeDelta::days(1);
    let plan = plan(
        &existing_rule(weekly(6)),
        &rows,
        &request(&rows[2], tuesday, None),
    )
    .unwrap();

    assert_eq!(plan.rule.recurrence, weekly(4));
    assert_eq!(moved_ids(&plan), ids(&rows[2..]));
    assert!(plan.extras.is_empty() && plan.leftovers.is_empty());
}

#[test]
fn an_omitted_until_rule_keeps_until() {
    let until = week(3);
    let rule = Recurrence {
        count: None,
        until: Some(until),
        ..weekly(0)
    };
    let rows = sessions(4);
    let plan = plan(
        &existing_rule(rule.clone()),
        &rows,
        &request(&rows[0], week(0), None),
    )
    .unwrap();

    assert_eq!(plan.rule.recurrence, rule);
}

#[test]
fn a_later_start_can_push_the_last_slot_past_until() {
    let rule = Recurrence {
        count: None,
        until: Some(week(3)),
        ..weekly(0)
    };
    let rows = sessions(4);
    let plan = plan(
        &existing_rule(rule),
        &rows,
        &request(&rows[0], week(0) + TimeDelta::days(4), None),
    )
    .unwrap();

    assert_eq!(ids(&plan.leftovers), ids(&rows[3..]));
}

#[test]
fn earlier_occurrences_split_the_series_and_the_old_rule_ends_before_the_anchor() {
    let rows = sessions(5);
    let plan = plan(
        &existing_rule(weekly(5)),
        &rows,
        &request(&rows[2], week(2) + TimeDelta::hours(1), None),
    )
    .unwrap();

    let shortened = plan.split_from.as_ref().expect("the series splits");
    assert_eq!(shortened.recurrence.count, None);
    assert_eq!(
        shortened.recurrence.until,
        Some(week(2) - TimeDelta::seconds(1))
    );
    assert_eq!(shortened.start_at, first_slot());
    assert_eq!(shortened.slots().unwrap(), vec![week(0), week(1)]);
    assert_eq!(plan.rule.start_at, week(2) + TimeDelta::hours(1));
}

#[test]
fn the_first_session_of_a_series_replaces_the_rule_in_place() {
    let rows = sessions(3);
    let plan = plan(
        &existing_rule(weekly(3)),
        &rows,
        &request(&rows[0], week(0) + TimeDelta::hours(1), None),
    )
    .unwrap();

    assert_eq!(plan.split_from, None);
}

#[test]
fn rows_before_the_rule_start_do_not_split_the_series() {
    // A full reschedule keeps past rows linked while the rule starts at the first new slot.
    let past = session(first_slot() - TimeDelta::weeks(2));
    let rows: Vec<_> = std::iter::once(past).chain(sessions(3)).collect();
    let plan = plan(
        &existing_rule(weekly(3)),
        &rows,
        &request(&rows[1], week(0) + TimeDelta::hours(1), None),
    )
    .unwrap();

    assert_eq!(plan.split_from, None);
    assert_eq!(moved_ids(&plan), ids(&rows[1..]));
}

#[test]
fn the_anchor_is_addressed_by_its_original_slot() {
    // Moved earlier on its own: its date is before week 1, its slot is week 1.
    let mut rows = sessions(3);
    rows[1].date = week(1) - TimeDelta::days(2);
    let plan = plan(
        &existing_rule(weekly(3)),
        &rows,
        &request(&rows[1], week(1), None),
    )
    .unwrap();

    let shortened = plan.split_from.expect("the series splits");
    assert_eq!(shortened.slots().unwrap(), vec![week(0)]);
}

#[test]
fn a_session_outside_the_series_is_refused() {
    let rows = sessions(3);
    let stranger = session(week(1));
    let err = plan(
        &existing_rule(weekly(3)),
        &rows,
        &request(&stranger, week(1), None),
    )
    .unwrap_err();
    assert!(is_validation(&err));
}

#[test]
fn a_past_anchor_is_refused() {
    let rows = sessions(3);
    let err = plan_following(
        &existing_rule(weekly(3)),
        &rows,
        &request(&rows[0], week(2), None),
        chrono_tz::UTC,
        week(1),
    )
    .unwrap_err();
    assert!(is_validation(&err));
}

#[test]
fn a_past_start_is_refused() {
    let rows = sessions(3);
    let err = plan(
        &existing_rule(weekly(3)),
        &rows,
        &request(&rows[0], now() - TimeDelta::hours(1), None),
    )
    .unwrap_err();
    assert_eq!(
        err.error_kind,
        DomainErrorKind::Validation(RecurrenceError::StartAtInPast.message())
    );
}

#[test]
fn an_omitted_duration_keeps_the_series_duration_and_a_given_one_wins() {
    let rows = sessions(2);
    let kept = plan(
        &existing_rule(weekly(2)),
        &rows,
        &request(&rows[0], week(0), None),
    )
    .unwrap();
    let given = plan(
        &existing_rule(weekly(2)),
        &rows,
        &FollowingReschedule {
            duration: Some(Duration::from_minutes_unchecked(30)),
            ..request(&rows[0], week(0), None)
        },
    )
    .unwrap();

    assert_eq!(kept.rule.duration_minutes, 60);
    assert_eq!(given.rule.duration_minutes, 30);
}

#[test]
fn a_request_matching_the_sessions_changes_nothing() {
    let rows = sessions(3);
    let plan = plan(
        &existing_rule(weekly(3)),
        &rows,
        &request(&rows[1], week(1), None),
    )
    .unwrap();

    assert!(plan.changes_nothing());
}

#[test]
fn a_new_duration_or_slot_is_a_change() {
    let rows = sessions(3);
    let resized = plan(
        &existing_rule(weekly(3)),
        &rows,
        &FollowingReschedule {
            duration: Some(Duration::from_minutes_unchecked(45)),
            ..request(&rows[1], week(1), None)
        },
    )
    .unwrap();
    let moved = plan(
        &existing_rule(weekly(3)),
        &rows,
        &request(&rows[1], week(1) + TimeDelta::hours(1), None),
    )
    .unwrap();

    assert!(!resized.changes_nothing());
    assert!(!moved.changes_nothing());
}

/// An earlier session moved on its own past the anchor's slot stays on the old series, so
/// the shortened rule must still hold its slot or calendars lose that meeting.
#[test]
fn a_session_moved_ahead_of_the_anchor_keeps_its_slot_on_the_old_series() {
    // Week 2's session was moved to before week 1's, so by date it precedes the anchor.
    let mut rows = sessions(4);
    rows[2].date = week(1) - TimeDelta::days(1);
    rows.sort_by_key(|session| (session.date, session.id));
    let anchor = rows
        .iter()
        .find(|session| session.ical_recurrence_id == Some(week(1)))
        .unwrap()
        .clone();

    let plan = plan(
        &existing_rule(weekly(4)),
        &rows,
        &request(&anchor, week(1) + TimeDelta::hours(1), None),
    )
    .unwrap();

    let shortened = plan.split_from.expect("the series splits");
    assert_eq!(shortened.slots().unwrap(), vec![week(0), week(1), week(2)]);
    assert!(plan
        .moves
        .iter()
        .all(|(session, _)| session.ical_recurrence_id != Some(week(2))));
}

#[test]
fn every_affected_session_is_superseded_as_it_was_before_the_move() {
    let rows = sessions(4);
    let plan = plan(
        &existing_rule(weekly(4)),
        &rows,
        &request(&rows[1], week(1) + TimeDelta::hours(1), Some(weekly(2))),
    )
    .unwrap();

    assert_eq!(plan.superseded(), rows[1..].to_vec());
}
