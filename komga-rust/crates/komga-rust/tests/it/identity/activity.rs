use std::time::Duration;

use reqwest::header::AUTHORIZATION;
use reqwest::{Method, StatusCode};
use serde_json::Value;

use super::{basic_auth, create_user};
use crate::support::db::main_pool;
use crate::support::{ADMIN_EMAIL, TestServer, json};

const READER: &str = "reader@example.org";

/// A request as the reader, authenticated with HTTP Basic, which records an activity.
async fn as_reader(server: &TestServer, path: &str, password: &str) -> reqwest::Response {
    server
        .request(Method::GET, path)
        .header(AUTHORIZATION, basic_auth(READER, password))
        .send()
        .await
        .expect("Reader request failed")
}

async fn admin_get(server: &TestServer, path: &str) -> (StatusCode, Value) {
    let response = server
        .admin(Method::GET, path)
        .send()
        .await
        .expect("Request failed");
    (response.status(), json(response).await)
}

fn emails(page: &Value) -> Vec<&str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|activity| activity["email"].as_str().unwrap_or_default())
        .collect()
}

#[tokio::test]
async fn activity_is_listed_newest_first_for_admins_and_owners() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = TestServer::start_claimed(dir.path()).await;
    let failed = as_reader(&server, "/api/v2/users/me", "password").await;
    assert_eq!(failed.status(), StatusCode::UNAUTHORIZED);
    let reader = create_user(&server, READER, "password").await;
    as_reader(&server, "/api/v2/users/me", "password").await;

    let (status, all) = admin_get(&server, "/api/v2/users/authentication-activity?size=2").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all["totalElements"], 3);
    assert_eq!(emails(&all), [READER, READER]);
    let newest = &all["content"][0];
    assert_eq!(newest["userId"], reader.as_str());
    assert_eq!(newest["success"], true);
    assert_eq!(newest["source"], "Password");
    assert!(
        newest["dateTime"]
            .as_str()
            .is_some_and(|value| value.len() == 20 && value.ends_with('Z')),
        "{newest}"
    );
    assert_eq!(all["content"][1]["success"], false);
    assert_eq!(all["content"][1]["error"], "Bad credentials");
    assert_eq!(all["content"][1]["userId"], Value::Null);

    let (_, by_email) = admin_get(
        &server,
        "/api/v2/users/authentication-activity?unpaged=true&sort=email,desc&sort=unknown",
    )
    .await;
    assert_eq!(emails(&by_email), [READER, READER, ADMIN_EMAIL]);

    // The failed attempt, before the account existed, has the reader's email only.
    let mine = as_reader(
        &server,
        "/api/v2/users/me/authentication-activity",
        "password",
    )
    .await;
    let mine = json(mine).await;
    assert_eq!(mine["totalElements"], 3, "{mine}");
    assert_eq!(emails(&mine), [READER, READER, READER]);

    let forbidden = as_reader(&server, "/api/v2/users/authentication-activity", "password").await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    server.stop().await;
}

#[tokio::test]
async fn the_latest_activity_is_the_admins_or_the_users_own() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = TestServer::start_claimed(dir.path()).await;
    let reader = create_user(&server, READER, "password").await;
    let latest = format!("/api/v2/users/{reader}/authentication-activity/latest");

    let (status, _) = admin_get(&server, &latest).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    as_reader(&server, "/api/v2/users/me", "password").await;
    let (status, activity) = admin_get(&server, &latest).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(activity["userId"], reader.as_str());
    let (status, _) = admin_get(&server, &format!("{latest}?apikey_id=unknown")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = admin_get(
        &server,
        "/api/v2/users/unknown/authentication-activity/latest",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let own = as_reader(&server, &latest, "password").await;
    assert_eq!(own.status(), StatusCode::OK);
    let (_, admin) = admin_get(&server, "/api/v2/users/me").await;
    let admin_id = admin["id"].as_str().expect("Admin ID");
    let other = format!("/api/v2/users/{admin_id}/authentication-activity/latest");
    assert_eq!(
        as_reader(&server, &other, "password").await.status(),
        StatusCode::FORBIDDEN
    );
    server.stop().await;
}

#[tokio::test]
async fn deleting_a_user_removes_its_activity_by_id_and_email() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = TestServer::start_claimed(dir.path()).await;
    // Recorded with the email only, as the account does not exist yet.
    as_reader(&server, "/api/v2/users/me", "password").await;
    let reader = create_user(&server, READER, "password").await;
    as_reader(&server, "/api/v2/users/me", "password").await;

    let deleted = server
        .admin(Method::DELETE, &format!("/api/v2/users/{reader}"))
        .send()
        .await
        .expect("Delete request failed");
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let (_, all) = admin_get(
        &server,
        "/api/v2/users/authentication-activity?unpaged=true",
    )
    .await;
    assert_eq!(emails(&all), [ADMIN_EMAIL]);
    server.stop().await;
}

#[tokio::test]
async fn activity_older_than_a_month_is_removed_at_startup() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    TestServer::start_claimed(dir.path()).await.stop().await;
    let pool = main_pool(dir.path()).await;
    for (email, age) in [
        ("old@example.org", "-32 days"),
        ("recent@example.org", "-27 days"),
    ] {
        sqlx::query(
            "INSERT INTO AUTHENTICATION_ACTIVITY (EMAIL, SUCCESS, DATE_TIME) \
             VALUES (?, 0, strftime('%Y-%m-%d %H:%M:%f', 'now', ?))",
        )
        .bind(email)
        .bind(age)
        .execute(&pool)
        .await
        .expect("Failed to insert an activity");
    }

    let server = TestServer::start(dir.path()).await;
    let remaining = || async {
        sqlx::query_scalar::<_, String>("SELECT EMAIL FROM AUTHENTICATION_ACTIVITY ORDER BY EMAIL")
            .fetch_all(&pool)
            .await
            .expect("Failed to list activities")
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while remaining().await.contains(&"old@example.org".to_owned()) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "Old activity must be removed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(remaining().await, [ADMIN_EMAIL, "recent@example.org"]);
    server.stop().await;
}
