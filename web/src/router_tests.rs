use super::ApiDoc;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
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
