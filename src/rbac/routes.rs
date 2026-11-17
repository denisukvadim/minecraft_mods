use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{AppError, ErrorResponse, map_unique_violation};
use crate::state::AppState;
use crate::validation;

use super::dto::{
    CreatePermissionRequest, CreateRoleRequest, PermissionDto, ReplacePermissionsRequest, RoleDto,
    UpdateRoleRequest,
};
use super::service::{self, RoleRow};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/permissions",
            get(list_permissions).post(create_permission),
        )
        .route("/permissions/{id}", delete(delete_permission))
        .route("/roles", get(list_roles).post(create_role))
        .route(
            "/roles/{id}",
            get(get_role).patch(update_role).delete(delete_role),
        )
        .route("/roles/{id}/permissions", put(replace_role_permissions))
}

#[utoipa::path(
    get,
    path = "/api/v1/permissions",
    tag = "permissions",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Every permission defined in the system", body = Vec<PermissionDto>),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking permissions:read", body = ErrorResponse)
    )
)]
pub async fn list_permissions(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<PermissionDto>>, AppError> {
    service::require_permission(&state.db, auth.id, "permissions:read").await?;

    let permissions = sqlx::query_as::<_, PermissionDto>(
        "SELECT id, code, description, created_at FROM permissions ORDER BY code",
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(permissions))
}

#[utoipa::path(
    post,
    path = "/api/v1/permissions",
    tag = "permissions",
    security(("bearer_auth" = [])),
    request_body = CreatePermissionRequest,
    responses(
        (status = 201, description = "Permission created", body = PermissionDto),
        (status = 400, description = "Invalid code", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 409, description = "Permission code already exists", body = ErrorResponse)
    )
)]
pub async fn create_permission(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<CreatePermissionRequest>,
) -> Result<(StatusCode, Json<PermissionDto>), AppError> {
    service::require_admin(&state.db, auth.id).await?;

    let code = payload.code.trim().to_lowercase();
    validation::permission_code(&code)?;

    let description = payload
        .description
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if let Some(description) = &description {
        validation::description(description, 255)?;
    }

    let permission = sqlx::query_as::<_, PermissionDto>(
        r#"
        INSERT INTO permissions (code, description)
        VALUES ($1, $2)
        RETURNING id, code, description, created_at
        "#,
    )
    .bind(&code)
    .bind(&description)
    .fetch_one(&state.db)
    .await
    .map_err(|err| map_unique_violation(err, "permission code already exists"))?;

    Ok((StatusCode::CREATED, Json(permission)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/permissions/{id}",
    tag = "permissions",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Permission id")),
    responses(
        (status = 204, description = "Permission deleted"),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 404, description = "Permission not found", body = ErrorResponse)
    )
)]
pub async fn delete_permission(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    service::require_admin(&state.db, auth.id).await?;

    let deleted: Option<(Uuid,)> =
        sqlx::query_as("DELETE FROM permissions WHERE id = $1 RETURNING id")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;

    deleted
        .map(|_| StatusCode::NO_CONTENT)
        .ok_or(AppError::NotFound("permission"))
}

#[utoipa::path(
    get,
    path = "/api/v1/roles",
    tag = "roles",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Roles with their permission sets", body = Vec<RoleDto>),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking roles:read", body = ErrorResponse)
    )
)]
pub async fn list_roles(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<RoleDto>>, AppError> {
    service::require_permission(&state.db, auth.id, "roles:read").await?;
    Ok(Json(service::load_roles(&state.db).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/roles/{id}",
    tag = "roles",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Role id")),
    responses(
        (status = 200, description = "Role with its permission set", body = RoleDto),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking roles:read", body = ErrorResponse),
        (status = 404, description = "Role not found", body = ErrorResponse)
    )
)]
pub async fn get_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<RoleDto>, AppError> {
    service::require_permission(&state.db, auth.id, "roles:read").await?;
    Ok(Json(service::load_role(&state.db, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/roles",
    tag = "roles",
    security(("bearer_auth" = [])),
    request_body = CreateRoleRequest,
    responses(
        (status = 201, description = "Role created", body = RoleDto),
        (status = 400, description = "Invalid role name or unknown permission code", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 409, description = "Role name already exists", body = ErrorResponse)
    )
)]
pub async fn create_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<CreateRoleRequest>,
) -> Result<(StatusCode, Json<RoleDto>), AppError> {
    service::require_admin(&state.db, auth.id).await?;

    let name = payload.name.trim().to_owned();
    validation::role_name(&name)?;

    let description = payload
        .description
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if let Some(description) = &description {
        validation::description(description, 255)?;
    }

    let mut tx = state.db.begin().await?;
    let row: RoleRow = sqlx::query_as(
        r#"
        INSERT INTO roles (name, description, is_system)
        VALUES ($1, $2, false)
        RETURNING id, name, description, is_system, created_at
        "#,
    )
    .bind(&name)
    .bind(&description)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| map_unique_violation(err, "role name already exists"))?;

    service::replace_permissions_in_tx(&mut tx, row.id, &payload.permissions).await?;
    tx.commit().await?;

    let role = service::load_role(&state.db, row.id).await?;
    Ok((StatusCode::CREATED, Json(role)))
}

#[utoipa::path(
    patch,
    path = "/api/v1/roles/{id}",
    tag = "roles",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Role id")),
    request_body = UpdateRoleRequest,
    responses(
        (status = 200, description = "Role updated", body = RoleDto),
        (status = 400, description = "Invalid payload or unknown permission code", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 404, description = "Role not found", body = ErrorResponse),
        (status = 409, description = "Role name already exists", body = ErrorResponse)
    )
)]
pub async fn update_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateRoleRequest>,
) -> Result<Json<RoleDto>, AppError> {
    service::require_admin(&state.db, auth.id).await?;

    let name = payload.name.map(|value| value.trim().to_owned());
    if let Some(name) = &name {
        validation::role_name(name)?;
    }

    let description = payload.description.map(|value| value.trim().to_owned());
    if let Some(description) = &description {
        validation::description(description, 255)?;
    }

    let existing: RoleRow = sqlx::query_as(
        "SELECT id, name, description, is_system, created_at FROM roles WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound("role"))?;

    // Renaming a system role would make the boot seeder create a second
    // `admin`/`user` row on the next start, so the name stays fixed.
    if existing.is_system
        && let Some(name) = &name
        && name != &existing.name
    {
        return Err(AppError::Validation(
            "system role name cannot be changed".to_owned(),
        ));
    }

    let mut tx = state.db.begin().await?;
    let updated: Option<RoleRow> = sqlx::query_as(
        r#"
        UPDATE roles
        SET name = COALESCE($2::text, name),
            description = COALESCE($3::text, description)
        WHERE id = $1
        RETURNING id, name, description, is_system, created_at
        "#,
    )
    .bind(id)
    .bind(&name)
    .bind(&description)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| map_unique_violation(err, "role name already exists"))?;

    if updated.is_none() {
        return Err(AppError::NotFound("role"));
    }

    if let Some(permissions) = payload.permissions {
        service::replace_permissions_in_tx(&mut tx, id, &permissions).await?;
    }

    tx.commit().await?;
    Ok(Json(service::load_role(&state.db, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/roles/{id}",
    tag = "roles",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Role id")),
    responses(
        (status = 204, description = "Role deleted"),
        (status = 400, description = "System roles cannot be deleted", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 404, description = "Role not found", body = ErrorResponse)
    )
)]
pub async fn delete_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    service::require_admin(&state.db, auth.id).await?;

    let role: Option<(bool,)> = sqlx::query_as("SELECT is_system FROM roles WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;

    let Some((is_system,)) = role else {
        return Err(AppError::NotFound("role"));
    };
    if is_system {
        return Err(AppError::Validation(
            "system roles cannot be deleted".to_owned(),
        ));
    }

    sqlx::query("DELETE FROM roles WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/api/v1/roles/{id}/permissions",
    tag = "roles",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Role id")),
    request_body = ReplacePermissionsRequest,
    responses(
        (status = 200, description = "Permission set replaced", body = RoleDto),
        (status = 400, description = "Unknown permission code", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Admin role required", body = ErrorResponse),
        (status = 404, description = "Role not found", body = ErrorResponse)
    )
)]
pub async fn replace_role_permissions(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(payload): Json<ReplacePermissionsRequest>,
) -> Result<Json<RoleDto>, AppError> {
    service::require_admin(&state.db, auth.id).await?;
    service::load_role(&state.db, id).await?;
    service::replace_role_permissions(&state.db, id, &payload.permissions).await?;

    Ok(Json(service::load_role(&state.db, id).await?))
}
