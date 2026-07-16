use super::*;

// Deserialization: absent vs explicit null vs value. These prove the
// clearable distinction the whole feature depends on.

#[test]
fn deserialize_title_value_sets_some_some() {
    let params: UpdateParams =
        serde_json::from_str(r#"{"date":"2026-06-07T10:00:00","title":"Renamed"}"#).unwrap();
    assert_eq!(params.title, Some(Some("Renamed".to_string())));
}

#[test]
fn deserialize_title_null_sets_some_none() {
    let params: UpdateParams =
        serde_json::from_str(r#"{"date":"2026-06-07T10:00:00","title":null}"#).unwrap();
    assert_eq!(params.title, Some(None));
}

#[test]
fn deserialize_title_omitted_sets_none() {
    let params: UpdateParams = serde_json::from_str(r#"{"date":"2026-06-07T10:00:00"}"#).unwrap();
    assert_eq!(params.title, None);
}

// Map building: value sets, null clears, omitted is absent.

fn base_update_params() -> UpdateParams {
    UpdateParams {
        date: NaiveDate::from_ymd_opt(2026, 6, 7)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap(),
        duration_minutes: None,
        meeting_url: None,
        provider: None,
        title: None,
    }
}

#[test]
fn into_update_map_sets_title_value() {
    let map = UpdateParams {
        title: Some(Some("Renamed".into())),
        ..base_update_params()
    }
    .into_update_map();
    assert_eq!(map.get("title").unwrap(), "Renamed");
}

#[test]
fn into_update_map_clears_title_with_null() {
    let map = UpdateParams {
        title: Some(None),
        ..base_update_params()
    }
    .into_update_map();
    assert!(matches!(map.get_value("title"), Some(Value::String(None))));
}

#[test]
fn into_update_map_omits_title_when_unchanged() {
    let map = UpdateParams {
        title: None,
        ..base_update_params()
    }
    .into_update_map();
    assert!(map.get_value("title").is_none());
}

// CreateParams threads the title straight onto the model.

fn base_create_params() -> CreateParams {
    CreateParams {
        coaching_relationship_id: Id::new_v4(),
        date: NaiveDate::from_ymd_opt(2026, 6, 7)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap(),
        duration_minutes: None,
        meeting_url: None,
        provider: None,
        title: None,
    }
}

#[test]
fn into_model_carries_title_value() {
    let model = CreateParams {
        title: Some("Quarterly planning".into()),
        ..base_create_params()
    }
    .into_model();
    assert_eq!(model.title, Some("Quarterly planning".to_string()));
}

#[test]
fn into_model_leaves_title_none_when_absent() {
    let model = CreateParams {
        title: None,
        ..base_create_params()
    }
    .into_model();
    assert_eq!(model.title, None);
}
