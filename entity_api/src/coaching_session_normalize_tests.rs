use super::{
    normalize_title, normalize_title_in_update_map, validate_title_length,
    validate_title_length_in_update_map, MAX_TITLE_LEN,
};
use crate::error::EntityApiErrorKind;
use crate::mutate::UpdateMap;
use sea_orm::Value;

#[test]
fn normalize_title_blanks_to_none() {
    assert_eq!(normalize_title(Some("  ".into())), None);
    assert_eq!(normalize_title(Some("\t \n".into())), None);
}

#[test]
fn normalize_title_trims_and_keeps_content() {
    assert_eq!(
        normalize_title(Some("  Quarterly  ".into())),
        Some("Quarterly".to_string())
    );
    assert_eq!(
        normalize_title(Some("Keep".into())),
        Some("Keep".to_string())
    );
}

#[test]
fn normalize_title_none_stays_none() {
    assert_eq!(normalize_title(None), None);
}

#[test]
fn normalize_update_map_clears_whitespace_title() {
    let mut map = UpdateMap::new();
    map.insert(
        "title".to_string(),
        Some(Value::String(Some(Box::new("   ".to_string())))),
    );
    normalize_title_in_update_map(&mut map);
    assert!(matches!(map.get_value("title"), Some(Value::String(None))));
}

#[test]
fn normalize_update_map_trims_non_empty_title() {
    let mut map = UpdateMap::new();
    map.insert(
        "title".to_string(),
        Some(Value::String(Some(Box::new(" x ".to_string())))),
    );
    normalize_title_in_update_map(&mut map);
    match map.get_value("title") {
        Some(Value::String(Some(s))) => assert_eq!(s.as_str(), "x"),
        other => panic!("expected Value::String(Some(\"x\")), got {other:?}"),
    }
}

#[test]
fn normalize_update_map_leaves_explicit_clear_unchanged() {
    let mut map = UpdateMap::new();
    map.insert("title".to_string(), Some(Value::String(None)));
    normalize_title_in_update_map(&mut map);
    assert!(matches!(map.get_value("title"), Some(Value::String(None))));
}

#[test]
fn normalize_update_map_leaves_absent_key_absent() {
    let mut map = UpdateMap::new();
    normalize_title_in_update_map(&mut map);
    assert!(map.get_value("title").is_none());
}

#[test]
fn validate_title_length_allows_none_and_at_cap() {
    assert!(validate_title_length(None).is_ok());
    let at_cap = "x".repeat(MAX_TITLE_LEN);
    assert!(validate_title_length(Some(&at_cap)).is_ok());
}

#[test]
fn validate_title_length_rejects_over_cap() {
    let over = "x".repeat(MAX_TITLE_LEN + 1);
    match validate_title_length(Some(&over)).unwrap_err().error_kind {
        EntityApiErrorKind::TitleTooLong { max, actual } => {
            assert_eq!(max, MAX_TITLE_LEN);
            assert_eq!(actual, MAX_TITLE_LEN + 1);
        }
        other => panic!("expected TitleTooLong, got {other:?}"),
    }
}

#[test]
fn validate_title_length_counts_chars_not_bytes() {
    // Multi-byte chars at the cap pass (byte length would overcount).
    let at_cap = "é".repeat(MAX_TITLE_LEN);
    assert!(validate_title_length(Some(&at_cap)).is_ok());
}

#[test]
fn validate_update_map_passes_short_and_absent_and_clear() {
    let mut map = UpdateMap::new();
    assert!(validate_title_length_in_update_map(&map).is_ok());
    map.insert("title".to_string(), Some(Value::String(None)));
    assert!(validate_title_length_in_update_map(&map).is_ok());
    map.insert(
        "title".to_string(),
        Some(Value::String(Some(Box::new("ok".to_string())))),
    );
    assert!(validate_title_length_in_update_map(&map).is_ok());
}

#[test]
fn validate_update_map_rejects_over_cap() {
    let mut map = UpdateMap::new();
    map.insert(
        "title".to_string(),
        Some(Value::String(Some(Box::new("x".repeat(MAX_TITLE_LEN + 1))))),
    );
    assert!(matches!(
        validate_title_length_in_update_map(&map)
            .unwrap_err()
            .error_kind,
        EntityApiErrorKind::TitleTooLong { .. }
    ));
}
