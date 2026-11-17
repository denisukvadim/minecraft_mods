use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use std::collections::HashSet;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::auth::password;
use crate::error::{AppError, ErrorResponse, map_unique_violation};
use crate::pagination::Pagination;
use crate::rbac::dto::AccessResponse;
use crate::rbac::service as rbac;
use crate::state::AppState;
use crate::validation;

use super::dto::{CreateUserRequest, UpdateUserRequest, UserDetail, UserSummary};
use super::service;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users", get(list_users).post(create_user))
        .route(
            "/users/{id}",
            get(get_user).patch(update_user).delete(delete_user),
        )
        .route(
            "/users/{id}/roles/{role_id}",
            post(assign_role).delete(revoke_role),
        )
        .route(
            "/users/{id}/permissions/{permission_id}",
            post(grant_permission).delete(revoke_permission),
        )
}

#[utoipa::path(
    get,
    path = "/api/v1/users",
    tag = "users",
    security(("bearer_auth" = [])),
    params(Pagination),
    responses(
        (status = 200, description = "Users page", body = Vec<UserSummary>),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking users:read", body = ErrorResponse)
    )
)]
pub async fn list_users(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(pagination): Query<Pagination>,
) -> Result<Json<Vec<UserSummary>>, AppError> {
    rbac::require_permission(&state.db, auth.id, "users:read").await?;

    let (limit, offset) = pagination.normalize();
    let users = sqlx::query_as::<_, UserSummary>(
        r#"
        SELECT id, email, username, is_active, created_at
        FROM users
        ORDER BY created_at DESC
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(users))
}

#[utoipa::path(
    post,
    path = "/api/v1/users",
    tag = "users",
    security(("bearer_auth" = [])),
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = UserDetail),
        (status = 400, description = "Invalid payload, unknown role, or roles chosen by a non-admin", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking users:create", body = ErrorResponse),
        (status = 409, description = "Email or username already taken", body = ErrorResponse)
    )
)]
pub async fn create_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserDetail>), AppError> {
    let admin = rbac::is_admin(&state.db, auth.id).await?;

    // Administrators pick roles explicitly. Everybody else may only hand out
    // their own access, so the new account can never exceed the creator.
    if !admin {
        rbac::require_permission(&state.db, auth.id, "users:create").await?;
        if !payload.roles.is_empty() {
            return Err(AppError::Validation(
                "only administrators can choose roles when creating a user".to_owned(),
            ));
        }
    }

    let email = payload.email.trim().to_lowercase();
    let username = payload.username.trim().to_owned();
    validation::email(&email)?;
    validation::username(&username)?;
    validation::password(&payload.password)?;

    let role_names = if admin {
        let mut names: Vec<String> = payload
            .roles
            .iter()
            .map(|role| role.trim().to_owned())
            .filter(|role| !role.is_empty())
            .collect();
        names.sort_unstable();
        names.dedup();
        if names.is_empty() {
            names.push("user".to_owned());
        }
        names
    } else {
        Vec::new()
    };

    let password = payload.password;
    let password_hash = tokio::task::spawn_blocking(move || password::hash_password(&password))
        .await
        .map_err(|err| {
            AppError::Internal(anyhow::anyhow!("password hashing task failed: {err}"))
        })??;

    let mut tx = state.db.begin().await?;

    if admin {
        let found: Vec<String> = sqlx::query_scalar("SELECT name FROM roles WHERE name = ANY($1)")
            .bind(&role_names)
            .fetch_all(&mut *tx)
            .await?;

        if found.len() != role_names.len() {
            let found_set: HashSet<&str> = found.iter().map(String::as_str).collect();
            let missing: Vec<&str> = role_names
                .iter()
                .map(String::as_str)
                .filter(|name| !found_set.contains(*name))
                .collect();
            return Err(AppError::Validation(format!(
                "unknown roles: {}",
                missing.join(", ")
            )));
        }
    }

    let user_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO users (email, username, password_hash, is_active)
        VALUES ($1, $2, $3, COALESCE($4::boolean, true))
        RETURNING id
        "#,
    )
    .bind(&email)
    .bind(&username)
    .bind(&password_hash)
    .bind(payload.is_active)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| map_unique_violation(err, "email or username is already taken"))?;

    if admin {
        sqlx::query(
            r#"
            INSERT INTO user_roles (user_id, role_id)
            SELECT $1, id FROM roles WHERE name = ANY($2)
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(&role_names)
        .execute(&mut *tx)
        .await?;
    } else {
        // Copy the creator's roles and direct grants inside the same
        // transaction: the new account starts with exactly the same access,
        // and no role name (let alone `admin`) can be smuggled in.
        sqlx::query(
            r#"
            INSERT INTO user_roles (user_id, role_id)
            SELECT $1, role_id FROM user_roles WHERE user_id = $2
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(auth.id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO user_permissions (user_id, permission_id)
            SELECT $1, permission_id FROM user_permissions WHERE user_id = $2
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(auth.id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(service::load_user_detail(&state.db, user_id).await?),
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/users/{id}",
    tag = "users",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 200, description = "User with roles and permissions", body = UserDetail),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking users:read", body = ErrorResponse),
        (status = 404, description = "User not found", body = ErrorResponse)
    )
)]
pub async fn get_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<UserDetail>, AppError> {
    rbac::require_permission(&state.db, auth.id, "users:read").await?;
    Ok(Json(service::load_user_detail(&state.db, id).await?))
}

#[utoipa::path(
    patch,
    path = "/api/v1/users/{id}",
    tag = "users",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "User updated", body = UserDetail),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 404, description = "User not found", body = ErrorResponse),
        (status = 409, description = "Email or username already taken", body = ErrorResponse)
    )
)]
pub async fn update_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateUserRequest>,
) -> Result<Json<UserDetail>, AppError> {
    rbac::require_admin(&state.db, auth.id).await?;

    let email = payload.email.map(|value| value.trim().to_lowercase());
    if let Some(email) = &email {
        validation::email(email)?;
    }

    let username = payload.username.map(|value| value.trim().to_owned());
    if let Some(username) = &username {
        validation::username(username)?;
    }

    let password_hash = match payload.password {
        Some(password) => {
            validation::password(&password)?;
            Some(
                tokio::task::spawn_blocking(move || password::hash_password(&password))
                    .await
                    .map_err(|err| {
                        AppError::Internal(anyhow::anyhow!("password hashing task failed: {err}"))
                    })??,
            )
        }
        None => None,
    };

    let updated = sqlx::query_as::<_, UserSummary>(
        r#"
        UPDATE users
        SET email = COALESCE($2::text, email),
            username = COALESCE($3::text, username),
            password_hash = COALESCE($4::text, password_hash),
            is_active = COALESCE($5::boolean, is_active),
            updated_at = now()
        WHERE id = $1
        RETURNING id, email, username, is_active, created_at
        "#,
    )
    .bind(id)
    .bind(&email)
    .bind(&username)
    .bind(&password_hash)
    .bind(payload.is_active)
    .fetch_optional(&state.db)
    .await
    .map_err(|err| map_unique_violation(err, "email or username is already taken"))?;

    if updated.is_none() {
        return Err(AppError::NotFound("user"));
    }

    Ok(Json(service::load_user_detail(&state.db, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/users/{id}",
    tag = "users",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 204, description = "User deleted"),
        (status = 400, description = "Cannot delete your own account", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 404, description = "User not found", body = ErrorResponse)
    )
)]
pub async fn delete_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    rbac::require_admin(&state.db, auth.id).await?;

    if auth.id == id {
        return Err(AppError::Validation(
            "cannot delete your own account".to_owned(),
        ));
    }

    let deleted: Option<(Uuid,)> = sqlx::query_as("DELETE FROM users WHERE id = $1 RETURNING id")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;

    deleted
        .map(|_| StatusCode::NO_CONTENT)
        .ok_or(AppError::NotFound("user"))
}

#[utoipa::path(
    post,
    path = "/api/v1/users/{id}/roles/{role_id}",
    tag = "users",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("role_id" = Uuid, Path, description = "Role id")
    ),
    responses(
        (status = 200, description = "Role attached; effective access returned", body = AccessResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 404, description = "User or role not found", body = ErrorResponse)
    )
)]
pub async fn assign_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((user_id, role_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<AccessResponse>, AppError> {
    rbac::require_admin(&state.db, auth.id).await?;
    service::load_user_summary(&state.db, user_id).await?;
    rbac::load_role(&state.db, role_id).await?;

    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .bind(role_id)
        .execute(&state.db)
        .await?;

    Ok(Json(rbac::access_for_user(&state.db, user_id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/users/{id}/roles/{role_id}",
    tag = "users",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("role_id" = Uuid, Path, description = "Role id")
    ),
    responses(
        (status = 200, description = "Role detached; effective access returned", body = AccessResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 404, description = "User not found", body = ErrorResponse)
    )
)]
pub async fn revoke_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((user_id, role_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<AccessResponse>, AppError> {
    rbac::require_admin(&state.db, auth.id).await?;
    service::load_user_summary(&state.db, user_id).await?;

    sqlx::query("DELETE FROM user_roles WHERE user_id = $1 AND role_id = $2")
        .bind(user_id)
        .bind(role_id)
        .execute(&state.db)
        .await?;

    Ok(Json(rbac::access_for_user(&state.db, user_id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/users/{id}/permissions/{permission_id}",
    tag = "users",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("permission_id" = Uuid, Path, description = "Permission id")
    ),
    responses(
        (status = 200, description = "Permission granted directly to the user; effective access returned", body = AccessResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 404, description = "User or permission not found", body = ErrorResponse)
    )
)]
pub async fn grant_permission(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((user_id, permission_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<AccessResponse>, AppError> {
    rbac::require_admin(&state.db, auth.id).await?;
    service::load_user_summary(&state.db, user_id).await?;
    rbac::find_permission(&state.db, permission_id).await?;

    sqlx::query(
        r#"
        INSERT INTO user_permissions (user_id, permission_id)
        VALUES ($1, $2)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(user_id)
    .bind(permission_id)
    .execute(&state.db)
    .await?;

    Ok(Json(rbac::access_for_user(&state.db, user_id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/users/{id}/permissions/{permission_id}",
    tag = "users",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("permission_id" = Uuid, Path, description = "Permission id")
    ),
    responses(
        (status = 200, description = "Direct permission revoked; effective access returned", body = AccessResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 404, description = "User not found", body = ErrorResponse)
    )
)]
pub async fn revoke_permission(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((user_id, permission_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<AccessResponse>, AppError> {
    rbac::require_admin(&state.db, auth.id).await?;
    service::load_user_summary(&state.db, user_id).await?;

    sqlx::query("DELETE FROM user_permissions WHERE user_id = $1 AND permission_id = $2")
        .bind(user_id)
        .bind(permission_id)
        .execute(&state.db)
        .await?;

    Ok(Json(rbac::access_for_user(&state.db, user_id).await?))
}
