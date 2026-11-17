use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::rbac::dto::{PermissionDto, RoleSummary};

#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct UserSummary {
    pub id: Uuid,
    pub email: String,
    pub username: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserDetail {
    pub id: Uuid,
    pub email: String,
    pub username: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub roles: Vec<RoleSummary>,
    /// Permissions granted to this user directly, outside of any role.
    pub direct_permissions: Vec<PermissionDto>,
    /// Roles plus direct grants, flattened.
    pub effective_permissions: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    #[schema(example = "alex@example.com")]
    pub email: String,
    #[schema(example = "alex")]
    pub username: String,
    #[schema(example = "correct horse battery staple")]
    pub password: String,
    /// Roles attached on creation. Empty or omitted means the `user` role.
    /// Only administrators may choose roles: other callers get exactly their
    /// own roles and direct grants copied, so the account cannot exceed them.
    #[serde(default)]
    #[schema(example = "[\"user\"]")]
    pub roles: Vec<String>,
    /// Defaults to true. Set false to create a disabled account.
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    pub email: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub is_active: Option<bool>,
}
