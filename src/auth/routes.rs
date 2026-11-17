use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use uuid::Uuid;

use crate::error::{AppError, ErrorResponse};
use crate::rbac::service as rbac;
use crate::state::AppState;
use crate::validation;

use super::dto::{ChangePasswordRequest, LoginRequest, MeResponse, TokenResponse};
use super::extractor::AuthUser;
use super::{jwt, password};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/me", get(me))
        .route("/auth/password", put(change_password))
}

#[derive(sqlx::FromRow)]
struct LoginRow {
    id: Uuid,
    password_hash: String,
    is_active: bool,
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Access token issued", body = TokenResponse),
        (status = 401, description = "Invalid credentials or inactive account", body = ErrorResponse)
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<TokenResponse>, AppError> {
    let email = payload.email.trim().to_lowercase();

    let user = sqlx::query_as::<_, LoginRow>(
        r#"
        SELECT id, password_hash, is_active
        FROM users
        WHERE email = $1
        "#,
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await?;

    // The same 401 for an unknown email, a wrong password and an inactive
    // account avoids turning the endpoint into an account-enumeration oracle.
    let Some(user) = user else {
        return Err(AppError::Unauthorized);
    };
    if !user.is_active {
        return Err(AppError::Unauthorized);
    }

    let password = payload.password;
    let stored_hash = user.password_hash;
    let valid =
        tokio::task::spawn_blocking(move || password::verify_password(&password, &stored_hash))
            .await
            .map_err(|err| {
                AppError::Internal(anyhow::anyhow!("password verification task failed: {err}"))
            })?;

    if !valid {
        return Err(AppError::Unauthorized);
    }

    let (access_token, expires_at) = jwt::issue(&state.config, user.id)?;

    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer".to_owned(),
        expires_at,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    tag = "auth",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Current user with effective access", body = MeResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse)
    )
)]
pub async fn me(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<MeResponse>, AppError> {
    let roles = rbac::role_names_for_user(&state.db, auth.id).await?;
    let permissions = rbac::permission_codes_for_user(&state.db, auth.id).await?;

    Ok(Json(MeResponse {
        id: auth.id,
        email: auth.email,
        username: auth.username,
        roles,
        permissions,
    }))
}

#[utoipa::path(
    put,
    path = "/api/v1/auth/password",
    tag = "auth",
    security(("bearer_auth" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (status = 204, description = "Password changed; existing tokens stay valid until they expire"),
        (status = 400, description = "Invalid new password or incorrect current password", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse)
    )
)]
pub async fn change_password(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<ChangePasswordRequest>,
) -> Result<StatusCode, AppError> {
    validation::password(&payload.new_password)?;

    if payload.new_password == payload.current_password {
        return Err(AppError::Validation(
            "the new password must be different from the current one".to_owned(),
        ));
    }

    let stored_hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(auth.id)
        .fetch_one(&state.db)
        .await?;

    let current = payload.current_password;
    let matches =
        tokio::task::spawn_blocking(move || password::verify_password(&current, &stored_hash))
            .await
            .map_err(|err| {
                AppError::Internal(anyhow::anyhow!("password verification task failed: {err}"))
            })?;

    if !matches {
        // 400 rather than 401 on purpose: the token is valid, and the admin
        // panel signs the user out on any 401 from an API call.
        return Err(AppError::Validation(
            "current password is incorrect".to_owned(),
        ));
    }

    let new_password = payload.new_password;
    let password_hash = tokio::task::spawn_blocking(move || password::hash_password(&new_password))
        .await
        .map_err(|err| {
            AppError::Internal(anyhow::anyhow!("password hashing task failed: {err}"))
        })??;

    sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1")
        .bind(auth.id)
        .bind(&password_hash)
        .execute(&state.db)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
