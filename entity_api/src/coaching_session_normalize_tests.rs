use super::{normalize_title, normalize_title_in_update_map};
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
