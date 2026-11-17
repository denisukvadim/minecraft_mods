use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct PermissionDto {
    pub id: Uuid,
    pub code: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct RoleSummary {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoleDto {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// System roles (admin, user) are seeded at boot and cannot be deleted.
    pub is_system: bool,
    pub created_at: DateTime<Utc>,
    pub permissions: Vec<PermissionDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePermissionRequest {
    /// Format `resource:action`, for example `mods:publish`.
    #[schema(example = "mods:publish")]
    pub code: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRoleRequest {
    #[schema(example = "moderator")]
    pub name: String,
    pub description: Option<String>,
    /// Permission codes attached to the new role. Unknown codes reject the request.
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    /// When present, replaces the whole permission set of the role.
    pub permissions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReplacePermissionsRequest {
    /// Complete permission set; an empty list removes every permission.
    #[schema(example = "[\"mods:read\", \"mods:download\"]")]
    pub permissions: Vec<String>,
}

/// Effective access of one user, combining roles and direct grants.
#[derive(Debug, Serialize, ToSchema)]
pub struct AccessResponse {
    pub user_id: Uuid,
    pub roles: Vec<RoleSummary>,
    pub direct_permissions: Vec<PermissionDto>,
    pub effective_permissions: Vec<String>,
}
