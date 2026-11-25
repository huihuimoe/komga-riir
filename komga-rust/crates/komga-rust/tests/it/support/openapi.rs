//! Checks real responses against the baseline `komga/docs/openapi.json`.
//!
//! The document describes only what springdoc sees on controllers: framework errors (401, 403,
//! 415, and Spring default 400 bodies) are undocumented, so only documented responses can pass.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use jsonschema::Validator;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, Response};
use serde_json::Value;

static OPENAPI: LazyLock<Value> = LazyLock::new(|| {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../komga/docs/openapi.json"
    );
    let text = std::fs::read_to_string(path).expect("Baseline OpenAPI document must exist");
    let mut schema: Value =
        serde_json::from_str(&text).expect("Baseline OpenAPI document must be JSON");
    // LibraryDto.kt and compatibility-snapshots/rest/libraries-list-admin.json include null.
    // springdoc omitted nullable here; keep Kotlin's response contract, not that schema defect.
    schema["components"]["schemas"]["LibraryDto"]["properties"]["oneshotsDirectory"]["type"] =
        serde_json::json!(["string", "null"]);
    // PageDto.kt `sizeBytes: Long?`, null for the pages of PDF books.
    schema["components"]["schemas"]["PageDto"]["properties"]["sizeBytes"]["type"] =
        serde_json::json!(["integer", "null"]);
    // ReadListRequestMatchDto.kt `releaseDate: LocalDate?`, null for series without one.
    schema["components"]["schemas"]["ReadListRequestBookMatchSeriesDto"]["properties"]["releaseDate"]
        ["type"] = serde_json::json!(["string", "null"]);
    // BookDto.kt `readProgress: ReadProgressDto?` and `releaseDate: LocalDate?`, null for unread
    // and undated books.
    schema["components"]["schemas"]["BookDto"]["properties"]["readProgress"] = serde_json::json!(
        {"anyOf": [{"$ref": "#/components/schemas/ReadProgressDto"}, {"type": "null"}]}
    );
    schema["components"]["schemas"]["BookMetadataDto"]["properties"]["releaseDate"]["type"] =
        serde_json::json!(["string", "null"]);
    // SeriesDto.kt `ageRating: Int?`, `totalBookCount: Int?` and `releaseDate: LocalDate?`.
    let series_metadata = &mut schema["components"]["schemas"]["SeriesMetadataDto"]["properties"];
    for property in ["ageRating", "totalBookCount"] {
        series_metadata[property]["type"] = serde_json::json!(["integer", "null"]);
    }
    schema["components"]["schemas"]["BookMetadataAggregationDto"]["properties"]["releaseDate"]["type"] =
        serde_json::json!(["string", "null"]);
    // TransientBookDto `number: Float?` and `seriesId: String?`, null before analysis.
    let transient_book = &mut schema["components"]["schemas"]["TransientBookDto"]["properties"];
    transient_book["number"]["type"] = serde_json::json!(["number", "null"]);
    transient_book["seriesId"]["type"] = serde_json::json!(["string", "null"]);
    schema
});

static VALIDATORS: LazyLock<Mutex<HashMap<String, Arc<Validator>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Reads a response, panics unless it is documented for `method` on its path, and returns its
/// JSON body (`Null` when empty).
pub async fn checked(method: Method, response: Response) -> Value {
    let path = response.url().path().to_owned();
    let status = response.status().as_u16();
    let content_type = response.headers().get(CONTENT_TYPE).map(|value| {
        value
            .to_str()
            .expect("Content-Type must be ASCII")
            .to_owned()
    });
    let bytes = response.bytes().await.expect("Response body failed");
    let body: Option<Value> = (!bytes.is_empty())
        .then(|| serde_json::from_slice(&bytes).expect("Response body must be valid JSON"));
    if let Err(error) = check(
        &method,
        &path,
        status,
        content_type.as_deref(),
        body.as_ref(),
    ) {
        panic!("{error}");
    }
    body.unwrap_or(Value::Null)
}

/// Matches the operation, status (or `default`), and media type, then validates the body against
/// the documented schema. The error names the operation and the JSON pointer that failed.
fn check(
    method: &Method,
    path: &str,
    status: u16,
    content_type: Option<&str>,
    body: Option<&Value>,
) -> Result<(), String> {
    let template =
        path_template(path).ok_or_else(|| format!("{method} {path}: path is not documented"))?;
    let operation_pointer = format!(
        "/paths/{}/{}",
        escape(template),
        method.as_str().to_ascii_lowercase()
    );
    let location = format!("{method} {template} -> {status}");
    let responses = OPENAPI
        .pointer(&format!("{operation_pointer}/responses"))
        .ok_or_else(|| format!("{method} {template}: method is not documented"))?;
    let status_key = [status.to_string(), "default".to_owned()]
        .into_iter()
        .find(|key| responses.get(key).is_some())
        .ok_or_else(|| format!("{location}: status is not documented"))?;

    let Some(content) = responses[&status_key].get("content") else {
        return match body {
            None => Ok(()),
            Some(_) => Err(format!("{location}: documented without a body")),
        };
    };
    let content_type = content_type.ok_or_else(|| format!("{location}: no Content-Type"))?;
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let media_type = content
        .as_object()
        .expect("OpenAPI content must be an object")
        .keys()
        .find(|documented| media_type_matches(documented, &essence))
        .ok_or_else(|| format!("{location}: media type {essence} is not documented"))?;
    let body = body.ok_or_else(|| format!("{location} {essence}: body is missing"))?;
    let schema_pointer = format!(
        "{operation_pointer}/responses/{status_key}/content/{}/schema",
        escape(media_type)
    );
    validate(&schema_pointer, body).map_err(|error| format!("{location} {essence}: {error}"))
}

/// Picks the documented template matching every segment, preferring literal segments.
fn path_template(path: &str) -> Option<&'static str> {
    let segments: Vec<&str> = path.split('/').collect();
    OPENAPI["paths"]
        .as_object()
        .expect("OpenAPI paths must be an object")
        .keys()
        .filter(|template| {
            let parts: Vec<&str> = template.split('/').collect();
            parts.len() == segments.len()
                && parts
                    .iter()
                    .zip(&segments)
                    .all(|(part, segment)| part.starts_with('{') || part == segment)
        })
        .min_by_key(|template| template.matches('{').count())
        .map(String::as_str)
}

fn media_type_matches(documented: &str, actual: &str) -> bool {
    match documented.strip_suffix("/*") {
        Some("*") => true,
        Some(kind) => actual
            .split_once('/')
            .is_some_and(|(actual_kind, _)| actual_kind == kind),
        None => documented == actual,
    }
}

/// Validates against a schema inside the document so `#/components/...` references resolve.
fn validate(schema_pointer: &str, body: &Value) -> Result<(), String> {
    let validator = {
        let mut cache = VALIDATORS.lock().expect("Validator cache lock");
        Arc::clone(cache.entry(schema_pointer.to_owned()).or_insert_with(|| {
            let mut schema = OPENAPI.clone();
            schema["$ref"] = Value::String(format!("#{}", fragment(schema_pointer)));
            let compiled = jsonschema::draft202012::new(&schema)
                .unwrap_or_else(|error| panic!("Schema {schema_pointer} must compile: {error}"));
            Arc::new(compiled)
        }))
    };
    let errors: Vec<String> = validator
        .iter_errors(body)
        .map(|error| format!("at '{}': {error}", error.instance_path()))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Percent-encodes what a URI fragment cannot hold, such as the braces of path templates.
fn fragment(pointer: &str) -> String {
    let mut encoded = String::with_capacity(pointer.len());
    for byte in pointer.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/?".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// JSON Pointer token escaping (RFC 6901).
fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn user() -> Value {
        json!({
            "id": "0S25T16F0ABHN", "email": "a@example.com", "roles": ["USER"],
            "sharedAllLibraries": true, "sharedLibrariesIds": [],
            "labelsAllow": [], "labelsExclude": []
        })
    }

    #[test]
    fn documented_responses_pass() {
        let json = Some("application/json");
        assert_eq!(
            check(&Method::GET, "/api/v2/users/me", 200, json, Some(&user())),
            Ok(())
        );
        // `{id}` matches any segment; 204 has no body.
        assert_eq!(
            check(&Method::PATCH, "/api/v2/users/abc", 204, None, None),
            Ok(())
        );
        // `*/*` accepts any media type; the templated pointer must still resolve.
        let violations = json!({"violations": [{"fieldName": "password", "message": "m"}]});
        let path = "/api/v2/users/abc/password";
        assert_eq!(
            check(&Method::PATCH, path, 400, json, Some(&violations)),
            Ok(())
        );
    }

    #[test]
    fn mismatches_name_their_location() {
        let json = Some("application/json");
        let mut bad_user = user();
        bad_user["roles"] = json!(["USER", 1]);
        let error = check(&Method::GET, "/api/v2/users/me", 200, json, Some(&bad_user))
            .expect_err("A non-string role must fail");
        assert!(
            error.starts_with("GET /api/v2/users/me -> 200 application/json: at '/roles/1':"),
            "{error}"
        );

        let cases = [
            (
                Method::GET,
                "/api/v2/nothing",
                200,
                json,
                "path is not documented",
            ),
            (
                Method::PUT,
                "/api/v2/users/me",
                200,
                json,
                "method is not documented",
            ),
            (
                Method::GET,
                "/api/v2/users/me",
                403,
                json,
                "status is not documented",
            ),
            (
                Method::GET,
                "/api/v2/users/me",
                200,
                Some("text/html"),
                "media type text/html",
            ),
        ];
        for (method, path, status, content_type, expected) in cases {
            let error = check(&method, path, status, content_type, Some(&user()))
                .expect_err("Undocumented response must fail");
            assert!(error.contains(expected), "{error}");
        }
    }
}
