use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::auth::AuthUser;
use crate::error::{AppError, ErrorResponse};
use crate::rbac::service as rbac;
use crate::server::client::PortainerClient;
use crate::state::AppState;

use super::dto::{LifecycleQuery, LogsQuery, ServerLogsDto, ServerStatusDto};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/server/status", get(status))
        .route("/server/start", post(start))
        .route("/server/stop", post(stop))
        .route("/server/restart", post(restart))
        .route("/server/logs", get(logs))
}

fn client(state: &AppState) -> Result<&PortainerClient, AppError> {
    state.portainer.as_deref().ok_or_else(|| {
        AppError::Unavailable(
            "Minecraft server control is not configured: set PORTAINER_URL and PORTAINER_API_KEY"
                .to_owned(),
        )
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/server/status",
    tag = "server",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "State of the Minecraft container", body = ServerStatusDto),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking server:manage", body = ErrorResponse),
        (status = 502, description = "Portainer request failed", body = ErrorResponse),
        (status = 503, description = "Portainer is not configured", body = ErrorResponse)
    )
)]
pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<ServerStatusDto>, AppError> {
    rbac::require_permission(&state.db, auth.id, "server:manage").await?;

    let status = client(&state)?.status().await?;
    Ok(Json(status))
}

#[utoipa::path(
    post,
    path = "/api/v1/server/start",
    tag = "server",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Container start requested; the state after the call", body = ServerStatusDto),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking server:manage", body = ErrorResponse),
        (status = 404, description = "Container not found", body = ErrorResponse),
        (status = 502, description = "Portainer request failed", body = ErrorResponse),
        (status = 503, description = "Portainer is not configured", body = ErrorResponse)
    )
)]
pub async fn start(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<ServerStatusDto>, AppError> {
    rbac::require_permission(&state.db, auth.id, "server:manage").await?;

    let status = client(&state)?.start().await?;
    tracing::info!(container = %status.container, "Minecraft container start requested");
    Ok(Json(status))
}

#[utoipa::path(
    post,
    path = "/api/v1/server/stop",
    tag = "server",
    security(("bearer_auth" = [])),
    params(LifecycleQuery),
    responses(
        (status = 200, description = "Container stopped gracefully; the state after the call", body = ServerStatusDto),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking server:manage", body = ErrorResponse),
        (status = 404, description = "Container not found", body = ErrorResponse),
        (status = 502, description = "Portainer request failed", body = ErrorResponse),
        (status = 503, description = "Portainer is not configured", body = ErrorResponse)
    )
)]
pub async fn stop(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<LifecycleQuery>,
) -> Result<Json<ServerStatusDto>, AppError> {
    rbac::require_permission(&state.db, auth.id, "server:manage").await?;

    let status = client(&state)?.stop(query.timeout()).await?;
    tracing::info!(container = %status.container, "Minecraft container stopped");
    Ok(Json(status))
}

#[utoipa::path(
    post,
    path = "/api/v1/server/restart",
    tag = "server",
    security(("bearer_auth" = [])),
    params(LifecycleQuery),
    responses(
        (status = 200, description = "Container restarted gracefully; the state after the call", body = ServerStatusDto),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking server:manage", body = ErrorResponse),
        (status = 404, description = "Container not found", body = ErrorResponse),
        (status = 502, description = "Portainer request failed", body = ErrorResponse),
        (status = 503, description = "Portainer is not configured", body = ErrorResponse)
    )
)]
pub async fn restart(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<LifecycleQuery>,
) -> Result<Json<ServerStatusDto>, AppError> {
    rbac::require_permission(&state.db, auth.id, "server:manage").await?;

    let status = client(&state)?.restart(query.timeout()).await?;
    tracing::info!(container = %status.container, "Minecraft container restarted");
    Ok(Json(status))
}

#[utoipa::path(
    get,
    path = "/api/v1/server/logs",
    tag = "server",
    security(("bearer_auth" = [])),
    params(LogsQuery),
    responses(
        (status = 200, description = "Tail of the container log (stdout and stderr)", body = ServerLogsDto),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking server:manage", body = ErrorResponse),
        (status = 404, description = "Container not found", body = ErrorResponse),
        (status = 502, description = "Portainer request failed", body = ErrorResponse),
        (status = 503, description = "Portainer is not configured", body = ErrorResponse)
    )
)]
pub async fn logs(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<LogsQuery>,
) -> Result<Json<ServerLogsDto>, AppError> {
    rbac::require_permission(&state.db, auth.id, "server:manage").await?;

    let logs = client(&state)?
        .logs(query.tail(), query.timestamps())
        .await?;
    Ok(Json(logs))
}
