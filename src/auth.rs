use std::sync::LazyLock;

use axum::{extract::FromRequestParts, http::{header, request::Parts, StatusCode}};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tracing::info;
use uuid::Uuid;

use crate::config::CONFIG;

pub static DECODING_KEY: LazyLock<DecodingKey> = LazyLock::new(|| {
    DecodingKey::from_secret(CONFIG.jwt_secret.as_bytes()).expect("JWT_SECRET is not a valid HMAC key")
});

#[derive(Deserialize)]
struct Claims {
    sub: Uuid,
}

/// The caller, authenticated by an HS256 bearer token whose `sub` claim is their user id.
pub struct AuthUser(pub Uuid);

impl<S: Send + Sync> FromRequestParts<S> for AuthUser {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let unauthorized = || (StatusCode::UNAUTHORIZED, "Unauthorized".to_string());

        let token = parts.headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            // The auth scheme is case-insensitive (RFC 7235)
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim())
            .ok_or_else(unauthorized)?;

        // Validation::new requires and checks `exp` by default
        let claims = decode::<Claims>(
            token,
            &DECODING_KEY,
            &Validation::new(Algorithm::HS256),
        )
        .map_err(|e| {
            info!("Rejected token: {}", e);
            unauthorized()
        })?
        .claims;

        Ok(AuthUser(claims.sub))
    }
}
