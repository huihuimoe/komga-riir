//! Mihon read progress of a read list (`ReadListController.getMihonReadProgressByReadListId`/
//! `updateMihonReadProgressByReadListId`).

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::{READ_LISTS, create_user, fixture, send};
use crate::support::{ADMIN_EMAIL, TestServer, json, openapi};

const LIMITED: &str = "limited@example.com";

fn route(read_list_id: &str) -> String {
    format!("{READ_LISTS}/{read_list_id}/read-progress/tachiyomi")
}

async fn get(server: &TestServer, email: Option<&str>, read_list_id: &str) -> Value {
    let response = send(server, email, Method::GET, &route(read_list_id), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    openapi::checked(Method::GET, response).await
}

async fn put(server: &TestServer, email: Option<&str>, read_list_id: &str, last: i32) {
    let body = json!({"lastBookRead": last});
    let response = send(
        server,
        email,
        Method::PUT,
        &route(read_list_id),
        Some(&body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    openapi::checked(Method::PUT, response).await;
}

fn counts(read: i32, unread: i32, index: i32) -> Value {
    json!({
        "booksCount": read + unread,
        "booksReadCount": read,
        "booksUnreadCount": unread,
        "booksInProgressCount": 0,
        "lastReadContinuousIndex": index,
    })
}

/// The books `email` completed.
async fn completed(pool: &SqlitePool, email: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT r.BOOK_ID FROM READ_PROGRESS r JOIN USER u ON u.ID = r.USER_ID \
         WHERE u.EMAIL = ? AND r.COMPLETED ORDER BY r.BOOK_ID",
    )
    .bind(email)
    .fetch_all(pool)
    .await
    .expect("Completed books")
}

#[tokio::test]
async fn leading_visible_books_are_read_for_one_user() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let body = json!({"name": "Mihon", "bookIds": [f.b1, f.a1, f.a2]});
    let created = json(send(&f.server, None, Method::POST, READ_LISTS, Some(&body)).await).await;
    let list = created["id"].as_str().expect("Read list ID");
    let access = json!({"sharedLibraries": {"all": false, "libraryIds": [f.library_a]}});
    create_user(&f.server, LIMITED, access).await;
    let mark = format!("/api/v1/books/{}/read-progress", f.b1);
    let completion = json!({"completed": true});
    let marked = send(&f.server, None, Method::PATCH, &mark, Some(&completion)).await;
    assert_eq!(marked.status(), StatusCode::NO_CONTENT);

    assert_eq!(get(&f.server, None, list).await, counts(1, 2, 1));
    // The book of library B is neither counted nor completed for a user who cannot see it.
    assert_eq!(get(&f.server, Some(LIMITED), list).await, counts(0, 2, 0));
    for _ in 0..2 {
        put(&f.server, Some(LIMITED), list, 1).await;
    }
    assert_eq!(
        completed(&f.pool, LIMITED).await,
        std::slice::from_ref(&f.a1)
    );
    assert_eq!(get(&f.server, Some(LIMITED), list).await, counts(1, 1, 1));
    assert_eq!(get(&f.server, None, list).await, counts(1, 2, 1));

    put(&f.server, None, list, 10).await;
    let mut all = vec![f.a1.clone(), f.a2.clone(), f.b1.clone()];
    all.sort();
    assert_eq!(completed(&f.pool, ADMIN_EMAIL).await, all);
    assert_eq!(get(&f.server, None, list).await, counts(3, 0, 3));
    f.server.stop().await;
}

#[tokio::test]
async fn invalid_unknown_and_hidden_read_lists_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let body = json!({"name": "Mihon", "bookIds": [f.a1]});
    let created = json(send(&f.server, None, Method::POST, READ_LISTS, Some(&body)).await).await;
    let list = created["id"].as_str().expect("Read list ID");
    let outsider = "outsider@example.com";
    let access = json!({"sharedLibraries": {"all": false, "libraryIds": []}});
    create_user(&f.server, outsider, access).await;

    // The body is validated before the read list is looked up.
    for (id, body) in [("0", json!({"lastBookRead": -1})), (list, json!({}))] {
        let response = send(&f.server, None, Method::PUT, &route(id), Some(&body)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    let update = json!({"lastBookRead": 1});
    for method in [Method::GET, Method::PUT] {
        for (email, id) in [(None, "0"), (Some(outsider), list)] {
            let response = send(&f.server, email, method.clone(), &route(id), Some(&update)).await;
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{method} {email:?}"
            );
        }
        let anonymous = f.server.request(method.clone(), &route(list)).json(&update);
        let anonymous = anonymous.send().await.expect("Anonymous request");
        assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED, "{method}");
    }
    assert!(completed(&f.pool, outsider).await.is_empty());
    f.server.stop().await;
}
