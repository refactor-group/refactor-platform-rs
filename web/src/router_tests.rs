use super::ApiDoc;
use serde_json::Value;
use utoipa::OpenApi;

const ROLE_PATH: &str = "/organizations/{organization_id}/users/{user_id}/role";

fn spec() -> Value {
    serde_json::to_value(ApiDoc::openapi()).expect("the derived spec must serialize")
}

/// Every `$ref` reachable from `value`, as bare schema names.
fn schema_refs(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(members) => {
            members
                .iter()
                .for_each(|(key, member)| match (key.as_str(), member.as_str()) {
                    ("$ref", Some(reference)) => found.push(
                        reference
                            .trim_start_matches("#/components/schemas/")
                            .to_string(),
                    ),
                    _ => schema_refs(member, found),
                })
        }
        Value::Array(items) => items.iter().for_each(|item| schema_refs(item, found)),
        _ => {}
    }
}

/// A handler absent from `paths(...)` is silently missing from the served spec and
/// produces no compile error, so the registration is worth pinning.
#[test]
fn the_role_path_serves_all_four_operations() {
    let spec = spec();
    let operations = spec["paths"][ROLE_PATH]
        .as_object()
        .expect("the role path must be in the served spec");

    ["get", "post", "put", "delete"]
        .iter()
        .for_each(|method| assert!(operations.contains_key(*method), "missing {method}"));
}

/// Named explicitly rather than swept recursively. A sweep of the role path also
/// picks up `Id`, `Version` and `DateTimeWithTimeZone`, which dangle for every
/// endpoint in the spec and are tracked separately; requiring this change to fix
/// them would be scope it does not own.
///
/// `Role` is the one that matters here. It is the field these endpoints exist to
/// convey, and an unresolvable reference leaves a consumer unable to see that the
/// permitted values are `User`, `Admin` and `SuperAdmin`. Deriving `ToSchema` is not
/// enough on its own; utoipa serves only what is registered.
#[test]
fn the_schemas_the_role_endpoints_publish_are_defined() {
    let spec = spec();
    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("the spec must define schemas");

    let missing: Vec<&str> = ["UpdateRoleParams", "domain.user_roles.Model", "Role"]
        .into_iter()
        .filter(|name| !schemas.contains_key(*name))
        .collect();

    assert!(
        missing.is_empty(),
        "the role endpoints publish schemas the spec does not define: {missing:?}"
    );
}

/// Guards the test above against becoming vacuous: it checks that `Role` is defined,
/// which is only worth checking while something still points at it.
#[test]
fn the_membership_schema_references_the_role_enum() {
    let mut referenced = Vec::new();
    schema_refs(
        &spec()["components"]["schemas"]["domain.user_roles.Model"],
        &mut referenced,
    );

    assert!(
        referenced.iter().any(|name| name == "Role"),
        "the membership must reference the Role enum: {referenced:?}"
    );
}
