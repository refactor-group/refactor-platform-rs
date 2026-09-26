use super::*;

fn params(q: &str) -> IndexParams {
    IndexParams {
        q: q.to_string(),
        types: None,
        organization_id: None,
        coaching_relationship_id: None,
        user_id: None,
        coaching_session_id: None,
        goal_id: None,
        goal_filter: None,
        status: None,
        topic_status: None,
        created_from: None,
        created_to: None,
        updated_from: None,
        updated_to: None,
        tz: None,
        limit: None,
        cursor: None,
        mode: None,
        keyword_weight: None,
    }
}

/// The stable discriminator of an `InvalidParam` rejection, or a panic — every
/// test below pins the exact `error` string the FE branches on.
fn discriminator(result: Result<Spec, Error>) -> &'static str {
    match result {
        Err(Error::Web(WebErrorKind::InvalidParam { error, .. })) => error,
        other => panic!("expected InvalidParam, got {other:?}"),
    }
}

#[test]
fn a_plain_query_compiles_with_defaults() {
    let spec = params("quarterly review").compile().expect("compiles");
    assert_eq!(spec.query, "quarterly review");
    assert_eq!(spec.limit, 25);
    assert_eq!(spec.types.len(), 5);
    assert!(spec.cursor.is_none());
}

#[test]
fn q_shorter_than_two_chars_after_trim_is_rejected() {
    assert_eq!(discriminator(params("  a  ").compile()), "query_too_short");
}

#[test]
fn q_is_silently_truncated_to_256_chars() {
    let long = "x".repeat(300);
    let spec = params(&long).compile().expect("compiles");
    assert_eq!(spec.q_raw.chars().count(), 256);
    assert_eq!(spec.query.chars().count(), 256);
}

#[test]
fn unknown_type_token_is_a_400() {
    let mut p = params("review");
    p.types = Some("goals,gaols".to_string());
    assert_eq!(discriminator(p.compile()), "unknown_type");
}

#[test]
fn inactive_type_tokens_are_silently_dropped() {
    let mut p = params("review");
    p.types = Some("goals,notes,users,organizations,transcripts".to_string());
    let spec = p.compile().expect("compiles");
    assert_eq!(spec.types, vec![HitType::Goal]);
}

#[test]
fn all_requested_types_dropped_means_search_nothing_not_everything() {
    let mut p = params("review");
    p.types = Some("users,organizations".to_string());
    let spec = p.compile().expect("compiles");
    assert!(spec.types.is_empty());
}

#[test]
fn unlinked_goal_filter_with_goal_id_is_contradictory() {
    let mut p = params("review");
    p.goal_filter = Some(GoalFilterParam::Unlinked);
    p.goal_id = Some(Id::new_v4());
    assert_eq!(discriminator(p.compile()), "contradictory_params");
}

#[test]
fn reserved_modes_are_rejected_until_their_phases_ship() {
    let mut p = params("review");
    p.mode = Some(Mode::Semantic);
    assert_eq!(discriminator(p.compile()), "mode_unavailable");
}

#[test]
fn keyword_weight_range_is_validated_before_the_mode_contradiction() {
    let mut p = params("review");
    p.keyword_weight = Some(1.5);
    assert_eq!(discriminator(p.compile()), "keyword_weight_out_of_range");

    let mut p = params("review");
    p.keyword_weight = Some(0.7);
    assert_eq!(discriminator(p.compile()), "contradictory_params");
}

#[test]
fn malformed_cursor_is_a_400() {
    let mut p = params("review");
    p.cursor = Some("definitely-not-a-cursor".to_string());
    assert_eq!(discriminator(p.compile()), "malformed_cursor");
}

#[test]
fn a_genuine_cursor_round_trips_through_compile() {
    let cursor = Cursor {
        score: 0.42,
        hit_type: HitType::Topic,
        id: Id::new_v4(),
    };
    let mut p = params("review");
    p.cursor = Some(cursor.encode());
    let spec = p.compile().expect("compiles");
    assert_eq!(spec.cursor, Some(cursor));
}

#[test]
fn limit_is_defaulted_and_clamped() {
    let mut p = params("review");
    p.limit = Some(500);
    assert_eq!(p.compile().expect("compiles").limit, 100);

    let mut p = params("review");
    p.limit = Some(0);
    assert_eq!(p.compile().expect("compiles").limit, 1);
}

#[test]
fn invalid_timezone_uses_the_existing_discriminated_400() {
    let mut p = params("review");
    p.tz = Some("Not/A/Timezone".to_string());
    assert!(matches!(
        p.compile(),
        Err(Error::Web(WebErrorKind::InvalidTimezone(v))) if v == "Not/A/Timezone"
    ));
}

#[test]
fn date_windows_are_half_open_in_the_supplied_timezone() {
    let mut p = params("review");
    p.created_from = Some(NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid date"));
    p.created_to = Some(NaiveDate::from_ymd_opt(2026, 7, 2).expect("valid date"));
    p.tz = Some("Europe/Berlin".to_string());
    let spec = p.compile().expect("compiles");
    // Berlin is UTC+2 in July: local midnight = 22:00 UTC the previous day.
    let from = spec.filters.created.from.expect("from set");
    let to = spec.filters.created.to_exclusive.expect("to set");
    assert_eq!(from.to_utc().to_string(), "2026-06-30 22:00:00 UTC");
    // to_exclusive is the start of the day AFTER created_to.
    assert_eq!(to.to_utc().to_string(), "2026-07-02 22:00:00 UTC");
}

#[test]
fn trailing_whitespace_survives_into_q_raw_for_the_prefix_opt_out() {
    let spec = params("quart ").compile().expect("compiles");
    assert_eq!(spec.q_raw, "quart ");
    assert_eq!(spec.query, "quart");
}
