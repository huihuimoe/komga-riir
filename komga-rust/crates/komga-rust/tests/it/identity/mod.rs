//! Identity and authentication integration tests.

use komga_db::Database;
use komga_service::{EventBus, IdentityService, SessionRegistry};
use komga_types::{ActivitySort, AuthenticationActivity, PageRequest, SortDirection, SortOrder};

mod activity;
mod api_keys;
mod auth;
mod claim;
mod mixed_credentials;
mod oauth2;
mod oauth2_flow;
mod oauth2_sign_in;
mod remember_me;
mod request_bodies;
mod user_requests;
mod user_restrictions;
mod users;

/// Reads the identity state persisted by a server running over `config_dir`.
async fn persisted_identity(config_dir: &std::path::Path) -> IdentityService {
    let database = Database::connect(&config_dir.join("database.sqlite"))
        .await
        .expect("Failed to open the server database");
    IdentityService::new(database, SessionRegistry::new(), EventBus::default())
}

/// Every recorded authentication activity, newest first.
async fn activities(identity: &IdentityService) -> Vec<AuthenticationActivity> {
    let newest_first = [SortOrder {
        key: ActivitySort::DateTime,
        direction: SortDirection::Desc,
    }];
    identity
        .activities(None, &newest_first, PageRequest::Unpaged)
        .await
        .expect("Failed to list authentication activity")
        .items
}

/// Creates a user without roles through the admin API and returns its ID.
async fn create_user(server: &crate::support::TestServer, email: &str, password: &str) -> String {
    let response = server
        .admin(reqwest::Method::POST, "/api/v2/users")
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await
        .expect("User creation request failed");
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    crate::support::json(response).await["id"]
        .as_str()
        .expect("Created user must have an ID")
        .to_owned()
}

/// An `Authorization` header value for HTTP Basic credentials.
fn basic_auth(email: &str, password: &str) -> String {
    use base64::Engine;
    let credentials =
        base64::engine::general_purpose::STANDARD.encode(format!("{email}:{password}"));
    format!("Basic {credentials}")
}

fn assert_tsid_id(value: &serde_json::Value) {
    let id = value.as_str().expect("Entity ID must be a JSON string");
    assert_eq!(id.len(), 13, "TSID must contain 13 characters");
    assert!(
        id.chars()
            .all(|character| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(character)),
        "TSID must use the Crockford base32 alphabet"
    );
}

fn assert_default_error(body: &serde_json::Value, status: u64, message: &str, path: &str) {
    let object = body
        .as_object()
        .expect("Error response must be JSON object");
    let mut fields = object.keys().map(String::as_str).collect::<Vec<_>>();
    fields.sort_unstable();
    assert_eq!(fields, ["error", "message", "path", "status", "timestamp"]);
    assert_eq!(body["status"], status);
    assert_eq!(body["error"], "Bad Request");
    assert_eq!(body["message"], message);
    assert_eq!(body["path"], path);
    let timestamp = body["timestamp"]
        .as_str()
        .expect("Error timestamp must be an RFC3339 string");
    assert!(
        timestamp.contains('T') && timestamp.ends_with('Z'),
        "Error timestamp must use UTC RFC3339 format"
    );
}

/// Sends a GET with exactly the given headers and no implicit credentials.
async fn get_with(
    server: &crate::support::TestServer,
    path: &str,
    headers: &[(&str, &str)],
) -> reqwest::Response {
    let mut request = server.request(reqwest::Method::GET, path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request.send().await.expect("Request failed")
}

/// The `name=value` pair of the response cookie `name`, ready for a `Cookie` header.
fn set_cookie(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{name}=")))
        .and_then(|value| value.split(';').next())
        .map(str::to_owned)
}

fn has_expired_remember_cookie(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| {
            value.starts_with(&format!("{}=", komga_api::REMEMBER_ME_COOKIE_NAME))
                && value.contains("Max-Age=0")
        })
}
