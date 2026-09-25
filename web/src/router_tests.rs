use super::ApiDoc;
use regex::Regex;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use utoipa::OpenApi;

const ROLE_PATH: &str = "/organizations/{organization_id}/users/{user_id}/role";
const TRANSCRIPT_PATH: &str =
    "/coaching_sessions/{coaching_session_id}/transcriptions/{transcription_id}";

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

/// The transcript download is the one operation whose two representations are only
/// visible in the spec: a consumer cannot discover the `text/plain` file or the
/// `speaker` values from the route alone.
#[test]
fn the_transcript_download_operation_is_served_with_its_schemas() {
    let spec = spec();
    let operation = &spec["paths"][TRANSCRIPT_PATH]["get"];
    assert!(
        operation.is_object(),
        "the transcript path must serve a get"
    );

    let content = &operation["responses"]["200"]["content"];
    ["application/json", "text/plain"].iter().for_each(|media| {
        assert!(
            content.get(*media).is_some(),
            "the 200 must offer {media}: {content}"
        );
    });

    let speaker = operation["parameters"]
        .as_array()
        .expect("the operation must declare parameters")
        .iter()
        .find(|parameter| parameter["name"] == "speaker")
        .expect("the speaker query parameter must be declared");

    let mut speaker_refs = Vec::new();
    schema_refs(speaker, &mut speaker_refs);
    assert_eq!(speaker_refs, ["SpeakerRole"]);

    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("the spec must define schemas");

    assert_eq!(
        schemas["SpeakerRole"]["enum"],
        serde_json::json!(["coach", "coachee"])
    );
    ["Speaker", "domain.transcription.WithSpeakers"]
        .iter()
        .for_each(|name| assert!(schemas.contains_key(*name), "missing schema {name}"));

    // Sweep the operation and the three schemas it publishes: an unresolvable ref
    // leaves a consumer unable to render the download at all.
    let mut referenced = Vec::new();
    schema_refs(operation, &mut referenced);
    [
        "SpeakerRole",
        "Speaker",
        "domain.transcription.WithSpeakers",
    ]
    .iter()
    .for_each(|name| schema_refs(&schemas[*name], &mut referenced));

    let dangling: Vec<&String> = referenced
        .iter()
        .filter(|name| !schemas.contains_key(name.as_str()))
        .collect();

    assert!(
        dangling.is_empty(),
        "the transcript download references schemas the spec does not define: {dangling:?}"
    );
}

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

/// A declared `request_body` must correspond to a handler that actually extracts one,
/// and a handler that extracts one must declare it. Drift here publishes a request
/// contract that the endpoint neither accepts nor requires.
#[test]
fn request_body_annotations_match_handler_signatures() {
    let attr_start = Regex::new(r"#\[utoipa::path\(").expect("valid regex");
    let handler = Regex::new(
        r"pub(?:\(crate\))? async fn (\w+)\(([\s\S]*?)\)\s*->\s*Result<impl IntoResponse",
    )
    .expect("valid regex");
    let request_body = Regex::new(r"request_body = ([^,\n]+)").expect("valid regex");

    let mut problems = Vec::new();
    for file in controller_sources() {
        let src = fs::read_to_string(&file).expect("controller source readable");
        let attrs: Vec<usize> = attr_start.find_iter(&src).map(|m| m.start()).collect();

        for caps in handler.captures_iter(&src) {
            let whole = caps.get(0).expect("match 0");
            let (name, sig) = (&caps[1], &caps[2]);

            // The annotation governing this handler is the last one opened before it.
            let Some(&start) = attrs.iter().rfind(|&&a| a < whole.start()) else {
                continue;
            };
            let Some(len) = src[start..].find("\n)]") else {
                continue;
            };
            let attr = &src[start..start + len];

            let declared = request_body.captures(attr).map(|c| c[1].trim().to_string());
            let extracts = sig.contains("Json(");

            match (declared, extracts) {
                (Some(body), false) => problems.push(format!(
                    "{}::{name} declares `request_body = {body}` but takes no Json extractor",
                    file.display()
                )),
                (None, true) => problems.push(format!(
                    "{}::{name} takes a Json extractor but declares no `request_body`",
                    file.display()
                )),
                _ => {}
            }
        }
    }

    assert!(
        problems.is_empty(),
        "request_body drift:\n{}",
        problems.join("\n")
    );
}

fn controller_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir)
            .expect("controller dir readable")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/controller");
    let mut out = Vec::new();
    walk(&root, &mut out);
    out
}
