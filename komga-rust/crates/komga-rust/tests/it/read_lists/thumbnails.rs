//! Read list posters: the mosaic, uploads, selection and deletion.

use image::{GenericImageView, ImageFormat};
use reqwest::header::{CACHE_CONTROL, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{READ_LISTS, create_user, fixture, send};
use crate::support::posters::{file, list_at, png, selected_ids, status_and_body};
use crate::support::tasks::wait_for_generations;
use crate::support::{TestServer, json, openapi};

async fn create(server: &TestServer, name: &str, book_ids: &[&str]) -> String {
    let body = json!({"name": name, "bookIds": book_ids});
    let created = json(send(server, None, Method::POST, READ_LISTS, Some(&body)).await).await;
    created["id"].as_str().expect("Read list ID").to_owned()
}

fn route(read_list_id: &str) -> String {
    format!("{READ_LISTS}/{read_list_id}/thumbnails")
}

async fn upload(server: &TestServer, read_list_id: &str, query: &str, poster: Vec<u8>) -> Value {
    let request = server.admin(Method::POST, &format!("{}{query}", route(read_list_id)));
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

/// The body of a successful image response as `email`, or as the admin without one.
async fn image(server: &TestServer, email: Option<&str>, path: &str) -> Vec<u8> {
    let response = send(server, email, Method::GET, path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    assert_eq!(response.headers()[CONTENT_TYPE], "image/jpeg");
    response.bytes().await.expect("Image").to_vec()
}

#[tokio::test]
async fn read_list_posters_are_uploaded_selected_and_deleted() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let list = create(&f.server, "Posters", &[&f.a1, &f.a2]).await;
    assert!(list_at(&f.server, &route(&list)).await.is_empty());

    let first = upload(&f.server, &list, "", png(120, 80)).await;
    assert_eq!(
        first,
        json!({
            "id": first["id"], "readListId": list, "type": "USER_UPLOADED", "selected": true,
            "mediaType": "image/png", "fileSize": png(120, 80).len(), "width": 120, "height": 80,
        })
    );
    let second = upload(&f.server, &list, "?selected=false", png(30, 20)).await;
    assert_eq!(second["selected"], false);
    assert_eq!(
        selected_ids(&list_at(&f.server, &route(&list)).await),
        [id(&first)]
    );
    let poster = format!("{READ_LISTS}/{list}/thumbnail");
    assert!(
        image(&f.server, None, &poster).await == png(120, 80),
        "the selected upload"
    );
    let by_id = format!("{}/{}", route(&list), id(&second));
    assert!(
        image(&f.server, None, &by_id).await == png(30, 20),
        "the upload by ID"
    );

    let admin = |method, suffix: String| {
        f.server
            .admin(method, &format!("{}/{suffix}", route(&list)))
    };
    let selection = admin(Method::PUT, format!("{}/selected", id(&second)));
    assert_eq!(status_and_body(selection).await.0, StatusCode::ACCEPTED);
    assert_eq!(
        selected_ids(&list_at(&f.server, &route(&list)).await),
        [id(&second)]
    );
    assert!(
        image(&f.server, None, &poster).await == png(30, 20),
        "the new selection"
    );

    // Deleting the selected poster selects the remaining one.
    let deletion = admin(Method::DELETE, id(&second).to_owned());
    assert_eq!(status_and_body(deletion).await.0, StatusCode::ACCEPTED);
    assert_eq!(
        selected_ids(&list_at(&f.server, &route(&list)).await),
        [id(&first)]
    );
    let deletion = admin(Method::DELETE, id(&first).to_owned());
    assert_eq!(status_and_body(deletion).await.0, StatusCode::ACCEPTED);
    assert!(list_at(&f.server, &route(&list)).await.is_empty());
    let mosaic = image(&f.server, None, &poster).await;
    let mosaic = image::load_from_memory_with_format(&mosaic, ImageFormat::Jpeg).expect("Mosaic");
    assert_eq!(mosaic.dimensions(), (212, 300), "a mosaic again");
    f.server.stop().await;
}

/// Without a selected poster, the thumbnail is a mosaic of the books the user sees, kept an hour.
#[tokio::test]
async fn mosaics_are_made_of_the_visible_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    wait_for_generations(directory.path()).await;
    // A distinct thumbnail for `b1`.
    let response = f
        .server
        .admin(Method::POST, &format!("/api/v1/books/{}/thumbnails", f.b1))
        .multipart(file(png(40, 60)))
        .send()
        .await
        .expect("Book poster upload");
    assert_eq!(response.status(), StatusCode::OK);
    let both = create(&f.server, "Both", &[&f.b1, &f.a1]).await;
    let a_only = create(&f.server, "A only", &[&f.a1]).await;
    let library_a = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.org", library_a).await;

    let path = format!("{READ_LISTS}/{both}/thumbnail");
    let response = send(&f.server, None, Method::GET, &path, None).await;
    assert_eq!(response.headers()[CACHE_CONTROL], "max-age=3600, private");
    let admin_mosaic = response.bytes().await.expect("Mosaic").to_vec();
    let restricted = image(&f.server, Some("a@example.org"), &path).await;
    let a_only_path = format!("{READ_LISTS}/{a_only}/thumbnail");
    assert!(
        restricted == image(&f.server, None, &a_only_path).await,
        "the hidden book is left out"
    );
    assert!(restricted != admin_mosaic, "the admin sees both books");
    let decoded = image::load_from_memory_with_format(&restricted, ImageFormat::Jpeg)
        .expect("Mosaic decodes");
    // The 48 x 20 page of `a1` in the first quadrant.
    let page = decoded.view(0, 0, 48, 20);
    assert!(
        page.pixels()
            .any(|(_, _, pixel)| pixel.0[..3].iter().any(|channel| *channel > 64)),
        "a book thumbnail, not black"
    );
    f.server.stop().await;
}

#[tokio::test]
async fn invalid_read_list_poster_requests_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let list_a = create(&f.server, "A", &[&f.a1]).await;
    let list_b = create(&f.server, "B", &[&f.b1]).await;
    let other = upload(&f.server, &list_b, "", png(8, 8)).await;
    let library_a = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.org", library_a).await;
    let as_user = |method, path: &str| {
        f.server
            .request(method, path)
            .basic_auth("a@example.org", Some("secret"))
    };
    let admin = |method, path: &str| f.server.admin(method, path);
    let foreign = format!("{}/{}", route(&list_a), id(&other));
    let poster_b = format!("{}/{}", route(&list_b), id(&other));
    let cases = [
        (
            admin(Method::POST, &route("missing")).multipart(file(png(8, 8))),
            StatusCode::NOT_FOUND,
        ),
        (
            admin(Method::POST, &route(&list_a)).multipart(file(b"text".to_vec())),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (admin(Method::GET, &foreign), StatusCode::BAD_REQUEST),
        (
            admin(Method::PUT, &format!("{foreign}/selected")),
            StatusCode::BAD_REQUEST,
        ),
        (admin(Method::DELETE, &foreign), StatusCode::BAD_REQUEST),
        (
            admin(Method::GET, &format!("{}/missing", route(&list_a))),
            StatusCode::NOT_FOUND,
        ),
        (
            admin(Method::PUT, &format!("{}/missing/selected", route(&list_a))),
            StatusCode::NOT_FOUND,
        ),
        (
            admin(Method::DELETE, &format!("{}/missing", route(&list_a))),
            StatusCode::NOT_FOUND,
        ),
        (
            admin(Method::GET, &format!("{READ_LISTS}/missing/thumbnail")),
            StatusCode::NOT_FOUND,
        ),
        (
            as_user(Method::POST, &route(&list_a)).multipart(file(png(8, 8))),
            StatusCode::FORBIDDEN,
        ),
        (
            as_user(Method::PUT, &format!("{poster_b}/selected")),
            StatusCode::FORBIDDEN,
        ),
        (as_user(Method::DELETE, &poster_b), StatusCode::FORBIDDEN),
        (as_user(Method::GET, &route(&list_a)), StatusCode::OK),
        (as_user(Method::GET, &route(&list_b)), StatusCode::NOT_FOUND),
        (as_user(Method::GET, &poster_b), StatusCode::NOT_FOUND),
        (
            as_user(Method::GET, &format!("{READ_LISTS}/{list_b}/thumbnail")),
            StatusCode::NOT_FOUND,
        ),
        (
            f.server.request(Method::GET, &route(&list_a)),
            StatusCode::UNAUTHORIZED,
        ),
    ];
    for (request, expected) in cases {
        let (code, body) = status_and_body(request).await;
        assert_eq!(code, expected, "{body}");
    }
    assert_eq!(
        selected_ids(&list_at(&f.server, &route(&list_b)).await),
        [id(&other)]
    );
    f.server.stop().await;
}
