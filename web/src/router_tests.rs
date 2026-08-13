use super::ApiDoc;
use utoipa::OpenApi;

/// Every `$ref` in the spec must point at a schema that is actually defined.
/// utoipa 4 silently emitted refs to unregistered types; this guards the regression.
#[test]
fn openapi_spec_has_no_dangling_refs() {
    let spec = ApiDoc::openapi().to_pretty_json().expect("spec serializes");
    let defined: Vec<String> = serde_json::from_str::<serde_json::Value>(&spec)
        .expect("spec parses")["components"]["schemas"]
        .as_object()
        .expect("schemas object")
        .keys()
        .cloned()
        .collect();

    let dangling: Vec<&str> = spec
        .match_indices("#/components/schemas/")
        .map(|(i, m)| {
            let rest = &spec[i + m.len()..];
            &rest[..rest.find('"').unwrap_or(0)]
        })
        .filter(|name| !defined.iter().any(|d| d == name))
        .collect();

    assert!(dangling.is_empty(), "dangling $refs: {dangling:?}");
}
