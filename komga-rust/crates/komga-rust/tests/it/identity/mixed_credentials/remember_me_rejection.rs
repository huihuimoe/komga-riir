use reqwest::header;
use reqwest::{Method, StatusCode};

use super::super::has_expired_remember_cookie;
use super::{
    assert_authenticated_as, credential_context, login_with_remember, send_request,
    tamper_cookie_value,
};

#[tokio::test]
async fn tampered_and_deleted_user_remember_tokens_are_rejected() {
    let context = credential_context().await;
    let (_, admin_remember) = login_with_remember(
        &context,
        &context.admin_basic,
        "203.0.113.20",
        "admin-client/1.0",
    )
    .await;
    let (_, reader_remember) = login_with_remember(
        &context,
        &context.reader_basic,
        "203.0.113.21",
        "reader-client/1.0",
    )
    .await;

    let tampered = send_request(
        &context,
        Method::GET,
        "/api/v2/users/me",
        vec![(
            header::COOKIE.as_str(),
            tamper_cookie_value(&admin_remember),
        )],
    )
    .await;
    assert_eq!(tampered.status(), StatusCode::UNAUTHORIZED);
    assert!(has_expired_remember_cookie(tampered.headers()));

    let deletion = send_request(
        &context,
        Method::DELETE,
        &format!("/api/v2/users/{}", context.reader_id),
        vec![(header::AUTHORIZATION.as_str(), context.admin_basic.clone())],
    )
    .await;
    assert_eq!(deletion.status(), StatusCode::NO_CONTENT);

    let deleted_user = send_request(
        &context,
        Method::GET,
        "/api/v2/users/me",
        vec![(header::COOKIE.as_str(), reader_remember)],
    )
    .await;
    assert_eq!(deleted_user.status(), StatusCode::UNAUTHORIZED);
    assert!(has_expired_remember_cookie(deleted_user.headers()));

    assert_authenticated_as(
        &context,
        vec![(header::COOKIE.as_str(), admin_remember)],
        crate::support::ADMIN_EMAIL,
    )
    .await;
}
