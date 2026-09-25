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
    let declares_body = Regex::new(r"request_body\s*[=(]").expect("valid regex");

    let problems: Vec<String> = sources_under(&["web/src/controller"])
        .iter()
        .flat_map(|file| {
            let src = fs::read_to_string(file).expect("controller source readable");
            annotated_handlers(&src)
                .into_iter()
                .filter_map(|handler| {
                    let Some((name, sig)) = handler.signature else {
                        return Some(format!(
                            "{}: a #[utoipa::path] annotation has no handler after it",
                            file.display()
                        ));
                    };
                    let declared = declares_body.is_match(handler.annotation);
                    let extracts = ["Json(", "Form(", "Multipart"]
                        .iter()
                        .any(|extractor| sig.contains(extractor));
                    match (declared, extracts) {
                        (true, false) => Some(format!(
                            "{}::{name} declares a request_body but extracts no body",
                            file.display()
                        )),
                        (false, true) => Some(format!(
                            "{}::{name} extracts a body but declares no request_body",
                            file.display()
                        )),
                        _ => None,
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();

    assert!(
        problems.is_empty(),
        "request_body drift:\n{}",
        problems.join("\n")
    );
}

/// utoipa keys `components/schemas` by the type's bare name unless `#[schema(as = ...)]`
/// overrides it, so two types sharing a name silently publish one schema for both.
#[test]
fn schema_names_are_unique_across_the_workspace() {
    let item = Regex::new(
        r"((?:\s*(?:#\[(?:[^\[\]]|\[(?:[^\[\]]|\[[^\[\]]*\])*\])*\]|//[^\n]*))*)\s*pub(?:\([a-z]+\))? (?:struct|enum) (\w+)",
    )
    .expect("valid regex");
    let derives_schema = Regex::new(r"derive\([^)]*\bToSchema\b").expect("valid regex");
    let schema_as = Regex::new(r"#\[schema\(as = ([\w:]+)").expect("valid regex");

    let mut names: Vec<(String, String)> =
        sources_under(&["entity/src", "domain/src", "service/src", "web/src"])
            .iter()
            .flat_map(|file| {
                let src = fs::read_to_string(file).expect("source readable");
                item.captures_iter(&src)
                    .filter(|caps| derives_schema.is_match(&caps[1]))
                    .map(|caps| {
                        let name = schema_as
                            .captures(&caps[1])
                            .map_or_else(|| caps[2].to_string(), |c| c[1].to_string());
                        (name, file.display().to_string())
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
    names.sort();

    let collisions: Vec<String> = names
        .windows(2)
        .filter(|pair| pair[0].0 == pair[1].0)
        .map(|pair| format!("{} in {} and {}", pair[0].0, pair[0].1, pair[1].1))
        .collect();

    assert!(
        collisions.is_empty(),
        "schema names shared by more than one type:\n{}",
        collisions.join("\n")
    );
}

/// Every `{param}` in a path template must be declared on each of its operations.
#[test]
fn every_path_template_parameter_is_declared() {
    let template = Regex::new(r"\{(\w+)\}").expect("valid regex");
    let spec = spec();

    let undeclared: Vec<String> = spec["paths"]
        .as_object()
        .expect("paths object")
        .iter()
        .flat_map(|(path, operations)| {
            let wanted: Vec<&str> = template
                .captures_iter(path)
                .map(|c| c.get(1).expect("group 1").as_str())
                .collect();
            operations
                .as_object()
                .expect("operations object")
                .iter()
                .flat_map(move |(method, operation)| {
                    let declared: Vec<&str> = operation["parameters"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|p| p["in"] == "path")
                        .filter_map(|p| p["name"].as_str())
                        .collect();
                    wanted
                        .clone()
                        .into_iter()
                        .filter(move |name| !declared.contains(name))
                        .map(move |name| format!("{method} {path}: {{{name}}}"))
                })
                .collect::<Vec<_>>()
        })
        .collect();

    assert!(
        undeclared.is_empty(),
        "undeclared path parameters:\n{}",
        undeclared.join("\n")
    );
}

/// One `#[utoipa::path]` annotation and the handler that follows it, if any.
struct AnnotatedHandler<'a> {
    annotation: &'a str,
    signature: Option<(&'a str, &'a str)>,
}

/// Pairs each annotation with the next `async fn`, provided no other annotation opens first.
fn annotated_handlers(src: &str) -> Vec<AnnotatedHandler<'_>> {
    let starts: Vec<usize> = src
        .match_indices("#[utoipa::path(")
        .map(|(i, _)| i)
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &start)| {
            let end = starts.get(n + 1).copied().unwrap_or(src.len());
            let Some(fn_at) = src[start..end].find("async fn ").map(|i| start + i) else {
                return AnnotatedHandler {
                    annotation: &src[start..end],
                    signature: None,
                };
            };
            let name_start = fn_at + "async fn ".len();
            let open = name_start
                + src[name_start..]
                    .find('(')
                    .expect("fn has a parameter list");
            AnnotatedHandler {
                annotation: &src[start..fn_at],
                signature: Some((&src[name_start..open], parameter_list(src, open))),
            }
        })
        .collect()
}

/// The text between the `(` at `open` and its matching `)`.
fn parameter_list(src: &str, open: usize) -> &str {
    let close = src[open..]
        .char_indices()
        .scan(0i32, |depth, (i, c)| {
            *depth += match c {
                '(' => 1,
                ')' => -1,
                _ => 0,
            };
            Some((i, *depth))
        })
        .find(|&(_, depth)| depth == 0)
        .map(|(i, _)| open + i)
        .expect("balanced parameter list");
    &src[open + 1..close]
}

/// Every `.rs` file under the given workspace-relative directories.
fn sources_under(dirs: &[&str]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("source dir readable").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("web lives in the workspace");
    let mut out = Vec::new();
    dirs.iter()
        .for_each(|dir| walk(&workspace.join(dir), &mut out));
    out
}
