use komga_api::SESSION_COOKIE_NAME;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json as body};

use super::{assert_default_error, assert_tsid_id, basic_auth, set_cookie};
use crate::support::{ADMIN_EMAIL, ADMIN_PASSWORD, TestServer, db, json};

const ME: &str = "/api/v2/users/me";
const USERS: &str = "/api/v2/users";

struct UserTestContext {
    server: TestServer,
    admin_id: String,
    admin_auth: String,
    reader_id: String,
    reader_auth: String,
    temp_dir: komga_test_support::TestDir,
}

/// A credential header sent with a request.
#[derive(Clone, Copy)]
enum Credential<'a> {
    Basic(&'a str),
    Cookie(&'a str),
}

#[tokio::test]
async fn test_user_management_crud_and_session_invalidation() {
    let context = user_test_context().await;
    test_user_crud_and_permissions(&context).await;
    test_role_and_password_permissions(&context).await;
    let session_cookie = test_role_change_invalidates_session(&context).await;
    let (self_password_auth, self_password_cookie) =
        test_self_password_change_preserves_session(&context, &session_cookie).await;
    let (new_auth, new_cookie) =
        test_admin_password_change(&context, &self_password_auth, &self_password_cookie).await;
    test_user_deletion(&context, &new_auth, &new_cookie).await;
    context.server.stop().await;
}

async fn call(
    server: &TestServer,
    method: Method,
    uri: &str,
    credential: Credential<'_>,
    json_body: Option<Value>,
) -> Response {
    let mut request = server.request(method, uri);
    request = match credential {
        Credential::Basic(value) => request.header("Authorization", value),
        Credential::Cookie(value) => request.header("Cookie", value),
    };
    if let Some(json_body) = json_body {
        request = request.json(&json_body);
    }
    request.send().await.expect("User API request failed")
}

async fn me_status(context: &UserTestContext, credential: Credential<'_>) -> StatusCode {
    call(&context.server, Method::GET, ME, credential, None)
        .await
        .status()
}

fn session_cookie(response: &Response) -> String {
    set_cookie(response, SESSION_COOKIE_NAME).expect("Authentication must create a session cookie")
}

async fn user_test_context() -> UserTestContext {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let admin_auth = basic_auth(ADMIN_EMAIL, ADMIN_PASSWORD);
    let admin = call(
        &server,
        Method::GET,
        ME,
        Credential::Basic(&admin_auth),
        None,
    )
    .await;
    let admin_id = json(admin).await["id"]
        .as_str()
        .expect("admin ID")
        .to_owned();

    let reader = body!({"email": "reader@example.com", "password": "readerpass", "roles": ["FILE_DOWNLOAD"]});
    let response = call(
        &server,
        Method::POST,
        USERS,
        Credential::Basic(&admin_auth),
        Some(reader),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let reader = json(response).await;
    assert_tsid_id(&reader["id"]);
    assert_eq!(reader["email"], "reader@example.com");

    UserTestContext {
        temp_dir,
        server,
        admin_id,
        admin_auth,
        reader_id: reader["id"].as_str().expect("reader ID").to_owned(),
        reader_auth: basic_auth("reader@example.com", "readerpass"),
    }
}

async fn test_user_crud_and_permissions(context: &UserTestContext) {
    let admin = Credential::Basic(&context.admin_auth);
    let duplicate =
        body!({"email": "reader@example.com", "password": "other", "roles": ["FILE_DOWNLOAD"]});
    let response = call(&context.server, Method::POST, USERS, admin, Some(duplicate)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = json(response).await;
    assert_default_error(&error, 400, "A user with this email already exists", USERS);

    let reader = Credential::Basic(&context.reader_auth);
    let response = call(&context.server, Method::GET, USERS, reader, None).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = call(&context.server, Method::GET, USERS, admin, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let users = json(response).await;
    assert_eq!(
        users.as_array().expect("User list must be an array").len(),
        2
    );
}

async fn test_role_and_password_permissions(context: &UserTestContext) {
    let requests = [
        (
            format!("{USERS}/{}", context.reader_id),
            &context.reader_auth,
            body!({"roles": ["ADMIN"]}),
        ),
        (
            format!("{USERS}/{}/password", context.admin_id),
            &context.reader_auth,
            body!({"password": "unauthorized"}),
        ),
        (
            format!("{USERS}/{}", context.admin_id),
            &context.admin_auth,
            body!({"roles": ["FILE_DOWNLOAD"]}),
        ),
    ];
    for (uri, auth, json_body) in requests {
        let response = call(
            &context.server,
            Method::PATCH,
            &uri,
            Credential::Basic(auth),
            Some(json_body),
        )
        .await;
        assert_forbidden(response).await;
    }
}

async fn test_role_change_invalidates_session(context: &UserTestContext) -> String {
    let reader = Credential::Basic(&context.reader_auth);
    let response = call(&context.server, Method::GET, ME, reader, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let old_cookie = session_cookie(&response);
    assert_eq!(
        me_status(context, Credential::Cookie(&old_cookie)).await,
        StatusCode::OK
    );

    let uri = format!("{USERS}/{}", context.reader_id);
    let roles = body!({"roles": ["PAGE_STREAMING"]});
    let admin = Credential::Basic(&context.admin_auth);
    let response = call(&context.server, Method::PATCH, &uri, admin, Some(roles)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let old_session = me_status(context, Credential::Cookie(&old_cookie)).await;
    assert_eq!(old_session, StatusCode::UNAUTHORIZED);

    let response = call(&context.server, Method::GET, ME, reader, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = session_cookie(&response);
    assert_eq!(
        json(response).await["roles"],
        body!(["PAGE_STREAMING", "USER"])
    );
    cookie
}

async fn test_self_password_change_preserves_session(
    context: &UserTestContext,
    existing_session_cookie: &str,
) -> (String, String) {
    let session = Credential::Cookie(existing_session_cookie);
    let password = body!({"password": "reader-self-password"});
    let uri = format!("{ME}/password");
    let response = call(
        &context.server,
        Method::PATCH,
        &uri,
        session,
        Some(password),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(me_status(context, session).await, StatusCode::OK);
    let old_password = me_status(context, Credential::Basic(&context.reader_auth)).await;
    assert_eq!(old_password, StatusCode::UNAUTHORIZED);

    let self_password_auth = basic_auth("reader@example.com", "reader-self-password");
    let credential = Credential::Basic(&self_password_auth);
    let response = call(&context.server, Method::GET, ME, credential, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = session_cookie(&response);
    (self_password_auth, cookie)
}

async fn test_admin_password_change(
    context: &UserTestContext,
    old_password_auth: &str,
    old_session_cookie: &str,
) -> (String, String) {
    let uri = format!("{USERS}/{}/password", context.reader_id);
    let password = body!({"password": "newreaderpassword"});
    let admin = Credential::Basic(&context.admin_auth);
    let response = call(&context.server, Method::PATCH, &uri, admin, Some(password)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    for credential in [
        Credential::Cookie(old_session_cookie),
        Credential::Basic(old_password_auth),
    ] {
        assert_eq!(
            me_status(context, credential).await,
            StatusCode::UNAUTHORIZED
        );
    }

    let new_reader_auth = basic_auth("reader@example.com", "newreaderpassword");
    let credential = Credential::Basic(&new_reader_auth);
    let response = call(&context.server, Method::GET, ME, credential, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = session_cookie(&response);
    (new_reader_auth, cookie)
}

async fn test_user_deletion(context: &UserTestContext, reader_auth: &str, reader_cookie: &str) {
    // Every seeded table references the user or its sync point, so a missed cleanup fails the
    // deletion on a foreign key.
    let pool = db::main_pool(context.temp_dir.path()).await;
    db::seed::user_owned_records(&pool, &context.reader_id).await;
    pool.close().await;
    let uri = format!("{USERS}/{}", context.reader_id);
    let admin = Credential::Basic(&context.admin_auth);
    let response = call(&context.server, Method::DELETE, &uri, admin, None).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    for credential in [
        Credential::Cookie(reader_cookie),
        Credential::Basic(reader_auth),
    ] {
        assert_eq!(
            me_status(context, credential).await,
            StatusCode::UNAUTHORIZED
        );
    }

    let uri = format!("{USERS}/{}", context.admin_id);
    let response = call(&context.server, Method::DELETE, &uri, admin, None).await;
    assert_forbidden(response).await;
}

/// `@PreAuthorize` denials reach Spring Security's `AccessDeniedHandlerImpl`, whatever the rule.
async fn assert_forbidden(response: Response) {
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let error = json(response).await;
    assert_eq!(error["error"], "Forbidden");
    assert_eq!(error["message"], "Forbidden");
}
