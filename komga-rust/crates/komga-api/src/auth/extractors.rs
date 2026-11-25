//! Axum extractors for authenticated users.

use std::ops::Deref;
use std::sync::Arc;

use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use komga_types::KomgaUser;

use super::RequestAuthContext;
use crate::error::ApiError;

/// Extractor ensuring the request is authenticated.
#[derive(Clone)]
pub struct Authenticated(pub Arc<KomgaUser>);

impl Deref for Authenticated {
    type Target = KomgaUser;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Extractor ensuring the request is authenticated and has Admin role.
#[derive(Clone)]
pub struct Admin(pub Arc<KomgaUser>);

impl Deref for Admin {
    type Target = KomgaUser;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> FromRequestParts<S> for Authenticated
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let Some(auth) = parts.extensions.get::<RequestAuthContext>() else {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Authentication context missing",
                parts.uri.path(),
            )
            .into_response());
        };
        Ok(Self(Arc::clone(&auth.user)))
    }
}

/// `Option<Authenticated>` is `None` for anonymous requests on routes that permit them.
impl<S> OptionalFromRequestParts<S> for Authenticated
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<RequestAuthContext>()
            .map(|auth| Self(Arc::clone(&auth.user))))
    }
}

impl<S> FromRequestParts<S> for Admin
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Authenticated(user) =
            <Authenticated as FromRequestParts<S>>::from_request_parts(parts, state).await?;
        if user.is_admin() {
            Ok(Self(user))
        } else {
            Err(ApiError::forbidden(parts.uri.path()).into_response())
        }
    }
}
