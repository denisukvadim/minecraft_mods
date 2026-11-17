use axum::extract::{DefaultBodyLimit, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use utoipa::OpenApi as _;
use utoipa::ToSchema;
use utoipa_swagger_ui::SwaggerUi;

use crate::auth;
use crate::error::{AppError, ErrorResponse};
use crate::mods;
use crate::openapi::ApiDoc;
use crate::rbac;
use crate::server;
use crate::state::AppState;
use crate::users;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    #[schema(example = "ok")]
    pub status: String,
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "meta",
    responses(
        (status = 200, description = "Service and database are reachable", body = HealthResponse),
        (status = 500, description = "Database check failed", body = ErrorResponse)
    )
)]
pub async fn health(State(state): State<AppState>) -> Result<Json<HealthResponse>, AppError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await?;

    Ok(Json(HealthResponse {
        status: "ok".to_owned(),
    }))
}

pub fn build_router(state: AppState) -> Router {
    let cors = if state.config.allow_any_origin {
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
            .allow_origin(state.config.cors_origins.clone())
            .allow_methods(Any)
            .allow_headers(Any)
    };

    let max_upload = state.config.mods_max_upload_bytes as usize;

    let api = Router::new()
        .merge(auth::routes::router())
        .merge(users::routes::router())
        .merge(rbac::routes::router())
        .merge(server::routes::router())
        // Only the mods routes accept a body this large; the limit keeps a
        // single upload from being buffered into memory without bound.
        .merge(mods::routes::router().layer(DefaultBodyLimit::max(max_upload)));

    Router::new()
        .route("/health", get(health))
        .nest("/api/v1", api)
        .with_state(state)
        .merge(SwaggerUi::new("/docs").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
}
