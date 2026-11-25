//! Collection posters: the mosaic, uploads, selection and deletion. Read lists share the routes,
//! which `read_lists::thumbnails` covers in more detail.

use image::{GenericImageView, ImageFormat};
use reqwest::header::{CACHE_CONTROL, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{COLLECTIONS, fixture};
use crate::read_lists::{create_user, send};
use crate::support::posters::{file, list_at, png, selected_ids, status_and_body};
use crate::support::tasks::wait_for_generations;
use crate::support::{TestServer, json, openapi};

async fn create(server: &TestServer, name: &str, series_ids: &[&str]) -> String {
    let body = json!({"name": name, "seriesIds": series_ids});
    let created = json(send(server, None, Method::POST, COLLECTIONS, Some(&body)).await).await;
    created["id"].as_str().expect("Collection ID").to_owned()
}

fn route(collection_id: &str) -> String {
    format!("{COLLECTIONS}/{collection_id}/thumbnails")
}

async fn upload(server: &TestServer, collection_id: &str, query: &str, poster: Vec<u8>) -> Value {
    let request = server.admin(Method::POST, &format!("{}{query}", route(collection_id)));
    let response = request
        .multipart(file(poster))
        .send()
        .await
        .expect("Upload");
    assert_eq!(response.status(), StatusCode::OK);
    openapi::checked(Method::POST, response).await
}

fn id(poster: &Value) -> &str {
    poster["id"].as_str().expect("Poster ID")
}

async fn image(server: &TestServer, email: Option<&str>, path: &str) -> Vec<u8> {
    let response = send(server, email, Method::GET, path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    assert_eq!(response.headers()[CONTENT_TYPE], "image/jpeg");
    response.bytes().await.expect("Image").to_vec()
}

#[tokio::test]
async fn collection_posters_are_uploaded_selected_and_deleted() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let collection = create(&f.server, "Posters", &[&f.a1, &f.a2]).await;
    let first = upload(&f.server, &collection, "", png(120, 80)).await;
    assert_eq!(
        first,
        json!({
            "id": first["id"], "collectionId": collection, "type": "USER_UPLOADED",
            "selected": true, "mediaType": "image/png", "fileSize": png(120, 80).len(),
            "width": 120, "height": 80,
        })
    );
    let second = upload(&f.server, &collection, "?selected=false", png(30, 20)).await;
    let poster = format!("{COLLECTIONS}/{collection}/thumbnail");
    assert!(
        image(&f.server, None, &poster).await == png(120, 80),
        "the selected upload"
    );

    let admin = |method, suffix: String| {
        f.server
            .admin(method, &format!("{}/{suffix}", route(&collection)))
    };
    let selection = admin(Method::PUT, format!("{}/selected", id(&second)));
    assert_eq!(status_and_body(selection).await.0, StatusCode::ACCEPTED);
    let by_id = format!("{}/{}", route(&collection), id(&second));
    assert!(
        image(&f.server, None, &by_id).await == png(30, 20),
        "the upload by ID"
    );
    assert!(
        image(&f.server, None, &poster).await == png(30, 20),
        "the new selection"
    );

    let deletion = admin(Method::DELETE, id(&second).to_owned());
    assert_eq!(status_and_body(deletion).await.0, StatusCode::ACCEPTED);
    let posters = list_at(&f.server, &route(&collection)).await;
    assert_eq!(selected_ids(&posters), [id(&first)], "the remaining one");
    f.server.stop().await;
}

/// Without a selected poster, the thumbnail is a mosaic of the series the user sees.
#[tokio::test]
async fn mosaics_are_made_of_the_visible_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    wait_for_generations(directory.path()).await;
    // A distinct thumbnail for series `SB`.
    let response = f
        .server
        .admin(Method::POST, &format!("/api/v1/series/{}/thumbnails", f.b))
        .multipart(file(png(40, 60)))
        .send()
        .await
        .expect("Series poster upload");
    assert_eq!(response.status(), StatusCode::OK);
    let both = create(&f.server, "Both", &[&f.b, &f.a1]).await;
    let a_only = create(&f.server, "A only", &[&f.a1]).await;
    let library_a = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.org", library_a).await;

    let path = format!("{COLLECTIONS}/{both}/thumbnail");
    let response = send(&f.server, None, Method::GET, &path, None).await;
    assert_eq!(response.headers()[CACHE_CONTROL], "max-age=3600, private");
    let admin_mosaic = response.bytes().await.expect("Mosaic").to_vec();
    let restricted = image(&f.server, Some("a@example.org"), &path).await;
    let a_only_path = format!("{COLLECTIONS}/{a_only}/thumbnail");
    assert!(
        restricted == image(&f.server, None, &a_only_path).await,
        "the hidden series is left out"
    );
    assert!(restricted != admin_mosaic, "the admin sees both series");
    let decoded =
        image::load_from_memory_with_format(&restricted, ImageFormat::Jpeg).expect("Mosaic");
    assert_eq!(decoded.dimensions(), (212, 300));
    // The 48 x 20 cover of `SA1` in the first quadrant.
    let cover = decoded.view(0, 0, 48, 20);
    assert!(
        cover
            .pixels()
            .any(|(_, _, pixel)| pixel.0[..3].iter().any(|channel| *channel > 64)),
        "a series thumbnail, not black"
    );
    f.server.stop().await;
}

#[tokio::test]
async fn invalid_collection_poster_requests_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let collection_a = create(&f.server, "A", &[&f.a1]).await;
    let collection_b = create(&f.server, "B", &[&f.b]).await;
    let other = upload(&f.server, &collection_b, "", png(8, 8)).await;
    let library_a = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.org", library_a).await;
    let as_user = |method, path: &str| {
        f.server
            .request(method, path)
            .basic_auth("a@example.org", Some("secret"))
    };
    let admin = |method, path: &str| f.server.admin(method, path);
    let foreign = format!("{}/{}", route(&collection_a), id(&other));
    let poster_b = format!("{}/{}", route(&collection_b), id(&other));
    let cases = [
        (
            admin(Method::POST, &route(&collection_a)).multipart(file(b"text".to_vec())),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (admin(Method::GET, &foreign), StatusCode::BAD_REQUEST),
        (
            admin(Method::PUT, &format!("{foreign}/selected")),
            StatusCode::BAD_REQUEST,
        ),
        (
            admin(Method::DELETE, &format!("{}/missing", route(&collection_a))),
            StatusCode::NOT_FOUND,
        ),
        (as_user(Method::DELETE, &poster_b), StatusCode::FORBIDDEN),
        (as_user(Method::GET, &route(&collection_a)), StatusCode::OK),
        (as_user(Method::GET, &poster_b), StatusCode::NOT_FOUND),
        (
            as_user(
                Method::GET,
                &format!("{COLLECTIONS}/{collection_b}/thumbnail"),
            ),
            StatusCode::NOT_FOUND,
        ),
    ];
    for (request, expected) in cases {
        let (code, body) = status_and_body(request).await;
        assert_eq!(code, expected, "{body}");
    }
    f.server.stop().await;
}
