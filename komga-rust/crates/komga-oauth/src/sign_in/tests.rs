use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::json;
use tokio::net::TcpListener;
use url::Url;

use super::*;
use crate::builder::{Builder, common_provider};
use crate::{ClientSecret, ProviderProperties, RegistrationProperties};

pub(crate) async fn serve(router: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

fn registration(mut builder: Builder, base: &str, scope: &str) -> ClientRegistration {
    builder.apply_registration(&RegistrationProperties {
        client_id: Some("gh+id".to_owned()),
        client_secret: Some(ClientSecret::new("s&cret".to_owned())),
        scope: Some(vec![scope.to_owned()]),
        ..RegistrationProperties::default()
    });
    builder.apply_provider(&ProviderProperties {
        token_uri: Some(format!("{base}/token")),
        user_info_uri: Some(format!("{base}/user")),
        ..ProviderProperties::default()
    });
    builder.build().unwrap()
}

fn github(base: &str, scope: &str) -> ClientRegistration {
    registration(common_provider("github").unwrap()("github"), base, scope)
}

async fn sign_in(registration: &ClientRegistration) -> Result<ExternalUser, AuthorizationError> {
    let (_, request) =
        registration.authorization_request(&Url::parse("https://komga.test").unwrap());
    registration
        .sign_in(&ProviderClient::new(), &request, "the-code")
        .await
}

fn form(body: &str) -> Map<String, Value> {
    url::form_urlencoded::parse(body.as_bytes())
        .map(|(key, value)| (key.into_owned(), Value::String(value.into_owned())))
        .collect()
}

/// Answers like GitHub, but only to the exchange a basic-auth client must make.
async fn token(headers: HeaderMap, body: String) -> (StatusCode, Json<Value>) {
    let basic = format!("Basic {}", STANDARD.encode("gh%2Bid:s%26cret"));
    let form = form(&body);
    let valid = headers["authorization"] == basic.as_str()
        && form["grant_type"] == "authorization_code"
        && form["code"] == "the-code"
        && form["redirect_uri"] == "https://komga.test/login/oauth2/code/github"
        && form["code_verifier"]
            .as_str()
            .is_some_and(|v| v.len() == 43)
        && !form.contains_key("client_secret");
    if valid {
        (
            StatusCode::OK,
            Json(json!({"access_token": "at", "token_type": "bearer"})),
        )
    } else {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "unexpected_token_request"})),
        )
    }
}

fn bearer(headers: &HeaderMap) -> StatusCode {
    if headers["authorization"] == "Bearer at" {
        StatusCode::OK
    } else {
        StatusCode::UNAUTHORIZED
    }
}

#[tokio::test]
async fn github_logins_fall_back_to_the_verified_primary_email() {
    let base =
        serve(
            Router::new()
                .route("/token", post(token))
                .route(
                    "/user",
                    get(|h: HeaderMap| async move {
                        (bearer(&h), Json(json!({"id": 1, "email": null})))
                    }),
                )
                .route(
                    "/user/emails",
                    get(|h: HeaderMap| async move {
                        (
                            bearer(&h),
                            Json(json!([
                                {"email": "old@example.com", "verified": true, "primary": false},
                                {"email": "new@example.com", "verified": false, "primary": true},
                                {"email": "main@example.com", "verified": true, "primary": true},
                            ])),
                        )
                    }),
                ),
        )
        .await;

    let user = sign_in(&github(&base, "user:email")).await;
    assert_eq!(
        user,
        Ok(ExternalUser::OAuth2 {
            email: Some("main@example.com".to_owned())
        })
    );
    let user = sign_in(&github(&base, "read:user")).await;
    assert_eq!(user, Ok(ExternalUser::OAuth2 { email: None }));
}

#[tokio::test]
async fn token_endpoint_failures_are_reported() {
    let base = serve(
        Router::new()
            .route(
                "/invalid-grant/token",
                post(|| async {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "invalid_grant"})),
                    )
                }),
            )
            .route(
                "/broken/token",
                post(|| async { (StatusCode::BAD_GATEWAY, "upstream down") }),
            )
            .route(
                "/mac/token",
                post(|| async { Json(json!({"access_token": "at", "token_type": "mac"})) }),
            )
            .route(
                "/empty/token",
                post(|| async { Json(json!({"token_type": "bearer"})) }),
            ),
    )
    .await;

    let error = |path: &str| {
        let registration = github(&format!("{base}/{path}"), "read:user");
        async move { sign_in(&registration).await.unwrap_err() }
    };
    assert_eq!(
        error("invalid-grant").await,
        AuthorizationError::Provider("invalid_grant".to_owned())
    );
    for path in ["broken", "mac", "empty"] {
        assert_eq!(
            error(path).await,
            AuthorizationError::InvalidTokenResponse,
            "{path}"
        );
    }
    let unreachable = github("http://127.0.0.1:1", "read:user");
    assert_eq!(
        sign_in(&unreachable).await.unwrap_err(),
        AuthorizationError::InvalidTokenResponse
    );
}

#[tokio::test]
async fn user_info_must_name_the_user() {
    let tokens = || async { Json(json!({"access_token": "at", "token_type": "Bearer"})) };
    let base = serve(
        Router::new()
            .route("/form/token", post(tokens))
            .route(
                "/form/user",
                post(|body: String| async move {
                    let named = form(&body)["access_token"] == "at";
                    Json(if named {
                        json!({"login": "reader", "email": "r@example.com"})
                    } else {
                        json!({})
                    })
                }),
            )
            .route("/anonymous/token", post(tokens))
            .route(
                "/anonymous/user",
                get(|| async { Json(json!({"email": "r@example.com"})) }),
            )
            .route("/denied/token", post(tokens))
            .route(
                "/denied/user",
                get(|| async { (StatusCode::UNAUTHORIZED, Json(json!({}))) }),
            ),
    )
    .await;
    let custom = |path: &str, attribute: Option<&str>| {
        let mut builder = Builder::new("idp");
        builder.apply_provider(&ProviderProperties {
            authorization_uri: Some(format!("{base}/authorize")),
            user_info_authentication_method: Some("form".to_owned()),
            user_name_attribute: attribute.map(str::to_owned),
            ..ProviderProperties::default()
        });
        builder.apply_registration(&RegistrationProperties {
            authorization_grant_type: Some("authorization_code".to_owned()),
            redirect_uri: Some("{baseUrl}/{action}/oauth2/code/{registrationId}".to_owned()),
            client_authentication_method: Some("client_secret_post".to_owned()),
            ..RegistrationProperties::default()
        });
        registration(builder, &format!("{base}/{path}"), "profile")
    };

    let user = sign_in(&custom("form", Some("login"))).await;
    assert_eq!(
        user,
        Ok(ExternalUser::OAuth2 {
            email: Some("r@example.com".to_owned())
        })
    );
    assert_eq!(
        sign_in(&custom("form", None)).await,
        Err(AuthorizationError::MissingUserNameAttribute)
    );
    for path in ["anonymous", "denied"] {
        assert_eq!(
            sign_in(&github(&format!("{base}/{path}"), "read:user")).await,
            Err(AuthorizationError::InvalidUserInfoResponse),
            "{path}"
        );
    }
}
