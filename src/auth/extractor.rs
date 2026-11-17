use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

use super::jwt;

/// The authenticated caller. Rejecting here keeps every handler free of
/// token parsing; the user is re-read from the database so deactivation
/// takes effect immediately instead of when the token expires.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub email: String,
    pub username: String,
}

#[derive(sqlx::FromRow)]
struct ActiveUserRow {
    id: Uuid,
    email: String,
    username: String,
    is_active: bool,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let raw_header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(AppError::Unauthorized)?;

        let token = raw_header
            .strip_prefix("Bearer ")
            .ok_or(AppError::Unauthorized)?;

        let user_id =
            jwt::decode_user_id(&state.config, token).map_err(|_| AppError::Unauthorized)?;

        let user = sqlx::query_as::<_, ActiveUserRow>(
            "SELECT id, email, username, is_active FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?;

        match user {
            Some(ActiveUserRow {
                id,
                email,
                username,
                is_active: true,
            }) => Ok(AuthUser {
                id,
                email,
                username,
            }),
            _ => Err(AppError::Unauthorized),
        }
    }
}
