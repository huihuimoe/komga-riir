//! Series posters without a series thumbnail: the thumbnail of the book the library's
//! `seriesCover` picks. Selected series thumbnails are covered with local artwork.

use std::io::Cursor;

use image::{ImageFormat, Rgb, RgbImage};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::books::library_with_books;
use crate::books::read_progress::mark;
use crate::books::reading::access::create_reader;
use crate::books::reading::fetch;
use crate::books::thumbnails::{assert_jpeg, book_with_first_page};
use crate::libraries::route;
use crate::support::TestServer;
use crate::support::db::content::book;
use crate::support::db::seed;
use crate::support::tasks::wait_for_generations;

fn page(width: u32, height: u32) -> Vec<u8> {
    let image = RgbImage::from_pixel(width, height, Rgb([0, 0, 200]));
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("Encode page");
    bytes
}

async fn set_series_cover(server: &TestServer, library: &Value, cover: &str) {
    let response = server
        .admin(Method::PATCH, &route(library))
        .json(&json!({"seriesCover": cover}))
        .send()
        .await
        .expect("Library update");
    assert_eq!(response.status(), StatusCode::NO_CONTENT, "{cover}");
}

#[tokio::test]
async fn series_without_a_thumbnail_show_the_book_their_library_picks() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let mut truncated = page(800, 400);
    truncated.truncate(truncated.len() / 2);
    // Book thumbnails tell the books apart: 300 x 150, 150 x 300 and 300 x 100.
    let (server, library, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/1.cbz", book_with_first_page(&page(800, 400))),
            ("Series/2.cbz", book_with_first_page(&page(400, 800))),
            ("Series/3.cbz", book_with_first_page(&page(600, 200))),
            ("Bad/bad.cbz", book_with_first_page(&truncated)),
        ],
    )
    .await;
    wait_for_generations(directory.path()).await;
    let (_, series_id) = book(&pool, "1").await;
    let thumbnail = format!("/api/v1/series/{series_id}/thumbnail");
    let get = || fetch(server.admin(Method::GET, &thumbnail));

    // Nobody has read anything, so the first unread book is the first.
    for (cover, size) in [
        ("FIRST", (300, 150)),
        ("LAST", (300, 100)),
        ("FIRST_UNREAD_OR_FIRST", (300, 150)),
        ("FIRST_UNREAD_OR_LAST", (300, 150)),
    ] {
        set_series_cover(&server, &library, cover).await;
        assert_jpeg(&get().await, size);
    }

    // A series whose books have no thumbnail has none.
    let (_, bad_series) = book(&pool, "bad").await;
    let bad = fetch(server.admin(
        Method::GET,
        &format!("/api/v1/series/{bad_series}/thumbnail"),
    ));
    assert_eq!(bad.await.status, StatusCode::NOT_FOUND);
    let unknown = fetch(server.admin(Method::GET, "/api/v1/series/0/thumbnail"));
    assert_eq!(unknown.await.status, StatusCode::NOT_FOUND);

    // Any user who sees the series gets its thumbnail; others do not.
    seed::series_age_rating(&pool, &series_id, 16).await;
    let allowed = json!({"roles": [], "ageRestriction": {"age": 16, "restriction": "ALLOW_ONLY"}});
    let reader = create_reader(&server, "reader@example.com", allowed).await;
    assert_eq!(
        fetch(reader.get(&server, &thumbnail)).await.status,
        StatusCode::OK
    );
    for (email, access) in [
        (
            "outsider@example.com",
            json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
        ),
        (
            "young@example.com",
            json!({"roles": [], "ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}}),
        ),
    ] {
        let denied = create_reader(&server, email, access).await;
        assert_eq!(
            fetch(denied.get(&server, &thumbnail)).await.status,
            StatusCode::FORBIDDEN,
            "{email}"
        );
    }
    let anonymous = server
        .request(Method::GET, &thumbnail)
        .send()
        .await
        .expect("Request");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    server.stop().await;
}

#[tokio::test]
async fn first_unread_series_covers_skip_completed_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    // Book thumbnails tell the books apart: 300 x 150, 150 x 300 and 300 x 100.
    let (server, library, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/1.cbz", book_with_first_page(&page(800, 400))),
            ("Series/2.cbz", book_with_first_page(&page(400, 800))),
            ("Series/3.cbz", book_with_first_page(&page(600, 200))),
        ],
    )
    .await;
    wait_for_generations(directory.path()).await;
    let (first, series_id) = book(&pool, "1").await;
    let thumbnail = format!("/api/v1/series/{series_id}/thumbnail");
    let reader = create_reader(&server, "reader@example.com", json!({"roles": []})).await;

    set_series_cover(&server, &library, "FIRST_UNREAD_OR_FIRST").await;
    mark(&server, &first, json!({"completed": true})).await;
    assert_jpeg(
        &fetch(server.admin(Method::GET, &thumbnail)).await,
        (150, 300),
    );
    assert_jpeg(&fetch(reader.get(&server, &thumbnail)).await, (300, 150));

    for name in ["2", "3"] {
        mark(
            &server,
            &book(&pool, name).await.0,
            json!({"completed": true}),
        )
        .await;
    }
    assert_jpeg(
        &fetch(server.admin(Method::GET, &thumbnail)).await,
        (300, 150),
    );
    set_series_cover(&server, &library, "FIRST_UNREAD_OR_LAST").await;
    assert_jpeg(
        &fetch(server.admin(Method::GET, &thumbnail)).await,
        (300, 100),
    );
    server.stop().await;
}
