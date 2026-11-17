use std::path::Path as FilePath;

use axum::body::Body;
use axum::extract::multipart::{Field, Multipart, MultipartError};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{delete, get};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{AppError, ErrorResponse};
use crate::pagination::Pagination;
use crate::rbac::service as rbac;
use crate::server;
use crate::state::AppState;

use super::dto::{ModFileActionDto, ModFileDto, UploadModForm};
use super::storage::{self, ModAction};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/mods", get(list_mods).post(upload_mod))
        .route("/mods/archive", get(download_archive))
        .route("/mods/{file_name}", delete(delete_mod))
        .route("/mods/{file_name}/download", get(download_mod))
        .route("/mods/{file_name}/history", get(file_history))
        .route("/mod-actions", get(list_actions))
}

#[utoipa::path(
    get,
    path = "/api/v1/mods",
    tag = "mods",
    security(("bearer_auth" = [])),
    params(Pagination),
    responses(
        (status = 200, description = "Files in MODS_DIR, sorted by name", body = Vec<ModFileDto>),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking mods:read", body = ErrorResponse)
    )
)]
pub async fn list_mods(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(pagination): Query<Pagination>,
) -> Result<Json<Vec<ModFileDto>>, AppError> {
    rbac::require_permission(&state.db, auth.id, "mods:read").await?;

    let files = storage::list(&state.config.mods_dir).await?;
    let (limit, offset) = pagination.normalize();

    // The directory has to be read in full to be sorted, so paging happens in
    // memory and only bounds the response.
    let page = files
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();

    Ok(Json(page))
}

#[utoipa::path(
    post,
    path = "/api/v1/mods",
    tag = "mods",
    security(("bearer_auth" = [])),
    request_body(content = inline(UploadModForm), content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "File stored in MODS_DIR", body = ModFileDto),
        (status = 400, description = "Missing `file` field or invalid file name", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking mods:upload", body = ErrorResponse),
        (status = 409, description = "A file with that name already exists, or the Minecraft server is running", body = ErrorResponse),
        (status = 413, description = "File exceeds MODS_MAX_UPLOAD_BYTES", body = ErrorResponse),
        (status = 502, description = "The Minecraft server state could not be checked", body = ErrorResponse)
    )
)]
pub async fn upload_mod(
    State(state): State<AppState>,
    auth: AuthUser,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<ModFileDto>), AppError> {
    rbac::require_permission(&state.db, auth.id, "mods:upload").await?;
    // Reject before the body is buffered: a running server owns its mods.
    server::ensure_server_stopped(state.portainer.as_deref()).await?;

    // Skip non-file form fields; the upload itself must carry a file name.
    let mut field = loop {
        let Some(candidate) = multipart.next_field().await.map_err(multipart_error)? else {
            return Err(AppError::Validation(
                "multipart field `file` with a file name is required".to_owned(),
            ));
        };
        if candidate.file_name().is_some() {
            break candidate;
        }
    };

    // Some clients send a full path in the multipart header; only the last
    // component is a file name.
    let raw_name = field.file_name().map(str::to_owned).unwrap_or_default();
    let base = raw_name.rsplit(['/', '\\']).next().unwrap_or(&raw_name);
    let file_name = storage::validate_file_name(base)?;
    let target = storage::resolve(&state.config.mods_dir, &file_name)?;

    if tokio::fs::symlink_metadata(&target).await.is_ok() {
        return Err(AppError::Conflict(format!(
            "file `{file_name}` already exists"
        )));
    }

    // Stream into a hidden temp file first: a half-written upload never
    // becomes visible, and the rename into place is atomic.
    let temp = state
        .config
        .mods_dir
        .join(format!(".upload-{}", Uuid::new_v4()));

    let written = match write_field(&mut field, &temp, state.config.mods_max_upload_bytes).await {
        Ok(written) => written,
        Err(err) => {
            remove_quietly(&temp).await;
            return Err(err);
        }
    };

    if let Err(err) = tokio::fs::rename(&temp, &target).await {
        remove_quietly(&temp).await;
        return Err(AppError::Io(err));
    }

    // Keep the audit trail and the filesystem in sync: if the log row cannot
    // be written, the stored file is removed again.
    if let Err(err) = storage::log_action(&state.db, auth.id, ModAction::Upload, &file_name).await {
        remove_quietly(&target).await;
        return Err(err);
    }

    let metadata = tokio::fs::metadata(&target).await?;

    Ok((
        StatusCode::CREATED,
        Json(ModFileDto {
            file_name,
            size_bytes: written,
            modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
        }),
    ))
}

async fn write_field(
    field: &mut Field<'_>,
    path: &FilePath,
    max_bytes: u64,
) -> Result<u64, AppError> {
    let mut file = tokio::fs::File::create_new(path).await?;
    let mut written: u64 = 0;

    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        written += chunk.len() as u64;
        if written > max_bytes {
            return Err(AppError::PayloadTooLarge(format!(
                "file exceeds MODS_MAX_UPLOAD_BYTES ({max_bytes} bytes)"
            )));
        }
        file.write_all(&chunk).await?;
    }

    file.flush().await?;
    file.sync_all().await?;

    Ok(written)
}

#[utoipa::path(
    get,
    path = "/api/v1/mods/{file_name}/download",
    tag = "mods",
    security(("bearer_auth" = [])),
    params(("file_name" = String, Path, description = "File name inside MODS_DIR")),
    responses(
        (status = 200, description = "Raw file contents", body = Vec<u8>, content_type = "application/octet-stream"),
        (status = 400, description = "Invalid file name", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking mods:download", body = ErrorResponse),
        (status = 404, description = "File not found", body = ErrorResponse)
    )
)]
pub async fn download_mod(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(file_name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    rbac::require_permission(&state.db, auth.id, "mods:download").await?;

    let path = storage::resolve(&state.config.mods_dir, &file_name)?;
    let metadata = storage::ensure_regular_file(&path).await?;

    // Logged before the stream starts so an aborted transfer still leaves a
    // trace, and a database failure stops the download instead of hiding it.
    storage::log_action(&state.db, auth.id, ModAction::Download, &file_name).await?;

    let file = tokio::fs::File::open(&path).await?;
    let body = Body::from_stream(ReaderStream::new(file));

    let disposition = HeaderValue::from_str(&storage::content_disposition(&file_name))
        .map_err(|err| AppError::Internal(anyhow::anyhow!("invalid file name header: {err}")))?;

    let headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        ),
        (header::CONTENT_DISPOSITION, disposition),
        (header::CONTENT_LENGTH, HeaderValue::from(metadata.len())),
        (
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
    ];

    Ok((StatusCode::OK, headers, body))
}

#[utoipa::path(
    get,
    path = "/api/v1/mods/archive",
    tag = "mods",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "ZIP archive with every mod file, built on the fly", content_type = "application/zip"),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking mods:download", body = ErrorResponse),
        (status = 404, description = "There are no mod files to archive", body = ErrorResponse)
    )
)]
pub async fn download_archive(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    rbac::require_permission(&state.db, auth.id, "mods:download").await?;

    let files = storage::list(&state.config.mods_dir).await?;
    if files.is_empty() {
        return Err(AppError::NotFound("mod file"));
    }

    // Built next to the mods: same filesystem and enough room; the leading dot
    // keeps the temporary archive out of listings and API downloads.
    let temp = state
        .config
        .mods_dir
        .join(format!(".archive-{}.zip", Uuid::new_v4()));
    let build_path = temp.clone();
    let mods_dir = state.config.mods_dir.clone();

    let built =
        tokio::task::spawn_blocking(move || storage::build_archive(&mods_dir, &files, &build_path))
            .await
            .map_err(|err| AppError::Internal(anyhow::anyhow!("archive task failed: {err}")))?;

    if let Err(err) = built {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(AppError::Internal(err));
    }

    let file = match tokio::fs::File::open(&temp).await {
        Ok(file) => file,
        Err(err) => {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(AppError::Io(err));
        }
    };
    let size = file.metadata().await.map_err(AppError::Io)?.len();

    // The open handle keeps the data readable after the link is gone, so even
    // a client that disconnects mid-transfer leaves no archive behind.
    if let Err(err) = tokio::fs::remove_file(&temp).await {
        tracing::warn!(path = %temp.display(), error = %err, "could not unlink temporary archive");
    }

    let name = format!("mods-{}.zip", Utc::now().format("%Y%m%d"));
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
        .map_err(|err| AppError::Internal(anyhow::anyhow!("invalid archive name header: {err}")))?;

    let headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/zip"),
        ),
        (header::CONTENT_DISPOSITION, disposition),
        (header::CONTENT_LENGTH, HeaderValue::from(size)),
        (
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
    ];

    Ok((
        StatusCode::OK,
        headers,
        Body::from_stream(ReaderStream::new(file)),
    ))
}

#[utoipa::path(
    delete,
    path = "/api/v1/mods/{file_name}",
    tag = "mods",
    security(("bearer_auth" = [])),
    params(("file_name" = String, Path, description = "File name inside MODS_DIR")),
    responses(
        (status = 204, description = "File deleted"),
        (status = 400, description = "Invalid file name or not a regular file", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking mods:delete", body = ErrorResponse),
        (status = 404, description = "File not found", body = ErrorResponse),
        (status = 409, description = "The Minecraft server is running", body = ErrorResponse),
        (status = 502, description = "The Minecraft server state could not be checked", body = ErrorResponse)
    )
)]
pub async fn delete_mod(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(file_name): Path<String>,
) -> Result<StatusCode, AppError> {
    rbac::require_permission(&state.db, auth.id, "mods:delete").await?;
    server::ensure_server_stopped(state.portainer.as_deref()).await?;

    let path = storage::resolve(&state.config.mods_dir, &file_name)?;
    storage::ensure_regular_file(&path).await?;

    tokio::fs::remove_file(&path).await?;
    storage::log_action(&state.db, auth.id, ModAction::Delete, &file_name).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/mods/{file_name}/history",
    tag = "mods",
    security(("bearer_auth" = [])),
    params(
        ("file_name" = String, Path, description = "File name inside MODS_DIR"),
        Pagination
    ),
    responses(
        (status = 200, description = "Who uploaded, downloaded or deleted this file, newest first", body = Vec<ModFileActionDto>),
        (status = 400, description = "Invalid file name", body = ErrorResponse),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking mods:read", body = ErrorResponse)
    )
)]
pub async fn file_history(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(file_name): Path<String>,
    Query(pagination): Query<Pagination>,
) -> Result<Json<Vec<ModFileActionDto>>, AppError> {
    rbac::require_permission(&state.db, auth.id, "mods:read").await?;

    // Validated, not resolved: history is kept after the file is deleted, so
    // the name is only used to filter the audit table.
    let file_name = storage::validate_file_name(&file_name)?;
    let (limit, offset) = pagination.normalize();

    let actions = sqlx::query_as::<_, ModFileActionDto>(
        r#"
        SELECT a.id, a.user_id, u.username, u.email AS user_email, a.action,
               a.file_name, a.created_at
        FROM mod_file_actions a
        LEFT JOIN users u ON u.id = a.user_id
        WHERE a.file_name = $1
        ORDER BY a.created_at DESC, a.id DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(&file_name)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(actions))
}

#[utoipa::path(
    get,
    path = "/api/v1/mod-actions",
    tag = "mods",
    security(("bearer_auth" = [])),
    params(Pagination),
    responses(
        (status = 200, description = "File actions, newest first", body = Vec<ModFileActionDto>),
        (status = 401, description = "Missing or invalid token", body = ErrorResponse),
        (status = 403, description = "Lacking audit:read", body = ErrorResponse)
    )
)]
pub async fn list_actions(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(pagination): Query<Pagination>,
) -> Result<Json<Vec<ModFileActionDto>>, AppError> {
    rbac::require_permission(&state.db, auth.id, "audit:read").await?;

    let (limit, offset) = pagination.normalize();
    let actions = sqlx::query_as::<_, ModFileActionDto>(
        r#"
        SELECT a.id, a.user_id, u.username, u.email AS user_email, a.action,
               a.file_name, a.created_at
        FROM mod_file_actions a
        LEFT JOIN users u ON u.id = a.user_id
        ORDER BY a.created_at DESC, a.id DESC
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(actions))
}

fn multipart_error(err: MultipartError) -> AppError {
    AppError::Validation(format!("invalid multipart body: {err}"))
}

async fn remove_quietly(path: &FilePath) {
    if let Err(err) = tokio::fs::remove_file(path).await {
        tracing::warn!(path = %path.display(), error = %err, "could not remove file");
    }
}
