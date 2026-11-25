use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{EncodingKey, Header};
use serde_json::json;

use super::*;
use crate::builder::Builder;
use crate::sign_in::tests::serve;
use crate::{ClientSecret, ExternalUser, ProviderProperties, RegistrationProperties};

struct SigningKey {
    kid: &'static str,
    pkcs8: Vec<u8>,
    jwk: Value,
}

impl SigningKey {
    fn generate(kid: &'static str) -> Self {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let x = URL_SAFE_NO_PAD.encode(pair.public_key().as_ref());
        let jwk = json!({"kty": "OKP", "crv": "Ed25519", "x": x, "kid": kid, "use": "sig"});
        Self {
            kid,
            pkcs8: pkcs8.as_ref().to_vec(),
            jwk,
        }
    }

    fn sign(&self, claims: &Value) -> String {
        let header = Header {
            kid: Some(self.kid.to_owned()),
            ..Header::new(Algorithm::EdDSA)
        };
        let key = EncodingKey::from_ed_der(&self.pkcs8);
        jsonwebtoken::encode(&header, claims, &key).unwrap()
    }
}

/// What the provider stub answers; tests replace parts of it between logins.
#[derive(Default)]
struct Provider {
    token: Mutex<Value>,
    keys: Mutex<Value>,
    user_info: Mutex<Value>,
    key_fetches: AtomicUsize,
}

type Stub = Arc<Provider>;
type Edit = fn(&mut Value);
type Sign<'a> = dyn Fn(&AuthorizationRequest) -> Option<String> + 'a;

async fn start(provider: Stub) -> ClientRegistration {
    let router = Router::new()
        .route(
            "/token",
            post(|State(p): State<Stub>| async move { Json(p.token.lock().unwrap().clone()) }),
        )
        .route(
            "/keys",
            get(|State(p): State<Stub>| async move {
                p.key_fetches.fetch_add(1, Ordering::SeqCst);
                Json(p.keys.lock().unwrap().clone())
            }),
        )
        .route(
            "/user",
            get(|State(p): State<Stub>| async move { Json(p.user_info.lock().unwrap().clone()) }),
        )
        .with_state(provider);
    let base = serve(router).await;
    let mut builder = Builder::new("idp");
    builder.apply_registration(&RegistrationProperties {
        client_id: Some("komga".to_owned()),
        client_secret: Some(ClientSecret::new("secret".to_owned())),
        authorization_grant_type: Some("authorization_code".to_owned()),
        redirect_uri: Some("{baseUrl}/login/oauth2/code/{registrationId}".to_owned()),
        scope: Some(vec!["openid".to_owned(), "email".to_owned()]),
        ..RegistrationProperties::default()
    });
    builder.apply_provider(&ProviderProperties {
        authorization_uri: Some(format!("{base}/authorize")),
        token_uri: Some(format!("{base}/token")),
        user_info_uri: Some(format!("{base}/user")),
        jwk_set_uri: Some(format!("{base}/keys")),
        issuer_uri: Some("https://idp.test".to_owned()),
        user_name_attribute: Some("sub".to_owned()),
        ..ProviderProperties::default()
    });
    builder.build().unwrap()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// The claims of a valid ID token for `request`, changed by `edit`.
fn claims(request: &AuthorizationRequest, edit: impl FnOnce(&mut Value)) -> Value {
    let mut claims = json!({
        "iss": "https://idp.test", "sub": "u-1", "aud": "komga",
        "iat": now(), "exp": now() + 300,
        "nonce": sha256(request.nonce().unwrap()), "email": "id@example.com",
    });
    edit(&mut claims);
    claims
}

/// Starts a login and answers its code with the ID token `sign` makes for its request.
async fn sign_in(
    registration: &ClientRegistration,
    client: &ProviderClient,
    provider: &Provider,
    sign: impl FnOnce(&AuthorizationRequest) -> Option<String>,
) -> Result<ExternalUser, AuthorizationError> {
    let (_, request) =
        registration.authorization_request(&Url::parse("https://komga.test").unwrap());
    let mut token = json!({"access_token": "at", "token_type": "Bearer"});
    if let Some(id_token) = sign(&request) {
        token["id_token"] = Value::String(id_token);
    }
    *provider.token.lock().unwrap() = token;
    registration.sign_in(client, &request, "code").await
}

fn provider(keys: &[&SigningKey], user_info: Value) -> Stub {
    let keys: Vec<Value> = keys.iter().map(|key| key.jwk.clone()).collect();
    Arc::new(Provider {
        keys: Mutex::new(json!({ "keys": keys })),
        user_info: Mutex::new(user_info),
        ..Provider::default()
    })
}

#[tokio::test]
async fn id_token_claims_win_over_matching_user_info() {
    let key = SigningKey::generate("a");
    let user_info = json!({"sub": "u-1", "email": "info@example.com", "email_verified": "true"});
    let stub = provider(&[&key], user_info);
    let registration = start(Arc::clone(&stub)).await;
    let client = ProviderClient::new();

    let user = sign_in(&registration, &client, &stub, |request| {
        Some(key.sign(&claims(request, |_| {})))
    })
    .await;
    assert_eq!(
        user,
        Ok(ExternalUser::Oidc {
            email: Some("id@example.com".to_owned()),
            email_verified: Some(true),
        })
    );

    *stub.user_info.lock().unwrap() = json!({"sub": "someone-else", "email": "x@example.com"});
    let user = sign_in(&registration, &client, &stub, |request| {
        Some(key.sign(&claims(request, |_| {})))
    })
    .await;
    assert_eq!(user, Err(AuthorizationError::InvalidUserInfoResponse));
}

#[tokio::test]
async fn foreign_or_tampered_id_tokens_are_rejected() {
    let key = SigningKey::generate("a");
    let stranger = SigningKey::generate("a");
    let stub = provider(&[&key], json!({"sub": "u-1"}));
    let registration = start(Arc::clone(&stub)).await;
    let client = ProviderClient::new();
    let edits: [(&str, Edit); 7] = [
        ("issuer", |c| c["iss"] = json!("https://other.test")),
        ("audience", |c| c["aud"] = json!("other")),
        ("expired", |c| c["exp"] = json!(now() - 120)),
        ("issued later", |c| c["iat"] = json!(now() + 120)),
        ("no iat", |c| drop(c.as_object_mut().unwrap().remove("iat"))),
        ("audiences without azp", |c| {
            c["aud"] = json!(["komga", "other"])
        }),
        ("other azp", |c| c["azp"] = json!("other")),
    ];
    for (case, edit) in edits {
        let user = sign_in(&registration, &client, &stub, |request| {
            Some(key.sign(&claims(request, edit)))
        })
        .await;
        assert_eq!(user, Err(AuthorizationError::InvalidIdToken), "{case}");
    }

    let cases: [(&str, AuthorizationError, &Sign); 4] = [
        ("missing", AuthorizationError::InvalidIdToken, &|_| None),
        ("unknown key", AuthorizationError::InvalidIdToken, &|r| {
            Some(stranger.sign(&claims(r, |_| {})))
        }),
        ("HMAC", AuthorizationError::InvalidIdToken, &|r| {
            let header = Header {
                kid: Some("a".to_owned()),
                ..Header::new(Algorithm::HS256)
            };
            Some(
                jsonwebtoken::encode(
                    &header,
                    &claims(r, |_| {}),
                    &EncodingKey::from_secret(b"secret").unwrap(),
                )
                .unwrap(),
            )
        }),
        ("other login", AuthorizationError::InvalidNonce, &|r| {
            Some(key.sign(&claims(r, |c| c["nonce"] = json!(sha256("replayed")))))
        }),
    ];
    for (case, expected, sign) in cases {
        let user = sign_in(&registration, &client, &stub, sign).await;
        assert_eq!(user, Err(expected), "{case}");
    }
}

#[tokio::test]
async fn rotated_signing_keys_are_fetched_when_first_seen() {
    let old = SigningKey::generate("old");
    let new = SigningKey::generate("new");
    let stub = provider(&[&old], json!({"sub": "u-1"}));
    let registration = start(Arc::clone(&stub)).await;
    let client = ProviderClient::new();
    let login = async |key: &SigningKey| {
        let token = |request: &AuthorizationRequest| Some(key.sign(&claims(request, |_| {})));
        sign_in(&registration, &client, &stub, token).await
    };

    assert!(login(&old).await.is_ok());
    assert!(login(&old).await.is_ok());
    assert_eq!(stub.key_fetches.load(Ordering::SeqCst), 1);

    *stub.keys.lock().unwrap() = json!({ "keys": [new.jwk] });
    assert!(login(&new).await.is_ok());
    assert_eq!(stub.key_fetches.load(Ordering::SeqCst), 2);
    assert_eq!(login(&old).await, Err(AuthorizationError::InvalidIdToken));
}
