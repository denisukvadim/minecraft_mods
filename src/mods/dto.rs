use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

/// One file in `MODS_DIR`.
#[derive(Debug, Serialize, ToSchema)]
pub struct ModFileDto {
    pub file_name: String,
    pub size_bytes: u64,
    pub modified_at: Option<DateTime<Utc>>,
}

/// Documentation-only view of the multipart form `POST /api/v1/mods` expects.
/// The field is rendered as a file picker in Swagger UI; the handler reads the
/// same `file` field from an Axum `Multipart` extractor, so it is never
/// constructed.
#[derive(Debug, ToSchema)]
pub struct UploadModForm {
    /// The mod archive (`.jar`, `.zip`, ...).
    #[schema(value_type = String, format = Binary)]
    #[allow(dead_code)]
    pub file: String,
}

/// One row of the audit trail written for every file action.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct ModFileActionDto {
    pub id: i64,
    /// Null once the account that performed the action has been deleted.
    pub user_id: Option<Uuid>,
    /// Joined live from `users`; not stored in the audit table.
    pub user_email: Option<String>,
    pub username: Option<String>,
    /// `upload`, `download` or `delete`.
    pub action: String,
    pub file_name: String,
    pub created_at: DateTime<Utc>,
}
