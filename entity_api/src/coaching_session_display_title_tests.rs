use super::compose_display_title;

#[test]
fn title_wins_over_topic_and_goal() {
    assert_eq!(
        compose_display_title(Some("Set Title"), Some("topic body"), Some("goal title")),
        Some("Set Title".to_string())
    );
}

#[test]
fn falls_through_to_topic_when_title_absent() {
    assert_eq!(
        compose_display_title(None, Some("topic body"), Some("goal title")),
        Some("topic body".to_string())
    );
}

#[test]
fn blank_title_falls_through_to_topic() {
    assert_eq!(
        compose_display_title(Some("   "), Some("topic body"), Some("goal title")),
        Some("topic body".to_string())
    );
}

#[test]
fn falls_through_to_goal_when_title_and_topic_absent() {
    assert_eq!(
        compose_display_title(None, None, Some("goal title")),
        Some("goal title".to_string())
    );
}

#[test]
fn empty_goal_title_treated_as_absent() {
    // Goal titles can be "" on the wire; an empty goal title must not win.
    assert_eq!(compose_display_title(None, None, Some("")), None);
}

#[test]
fn all_absent_yields_none() {
    assert_eq!(compose_display_title(None, None, None), None);
}

#[test]
fn all_blank_yields_none() {
    assert_eq!(
        compose_display_title(Some(" "), Some("\t"), Some("\n")),
        None
    );
}

#[test]
fn winning_tier_is_trimmed() {
    assert_eq!(
        compose_display_title(Some("  Padded  "), None, None),
        Some("Padded".to_string())
    );
}
