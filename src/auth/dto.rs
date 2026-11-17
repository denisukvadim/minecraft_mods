use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    #[schema(example = "steve@example.com")]
    pub email: String,
    #[schema(example = "correct horse battery staple")]
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TokenResponse {
    pub access_token: String,
    #[schema(example = "Bearer")]
    pub token_type: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    /// The password in use right now, re-checked before the change.
    #[schema(example = "correct horse battery staple")]
    pub current_password: String,
    /// Replacement password, 8-128 characters, different from the current one.
    #[schema(example = "a brand new passphrase")]
    pub new_password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MeResponse {
    pub id: Uuid,
    pub email: String,
    pub username: String,
    /// Names of the roles attached to the user.
    pub roles: Vec<String>,
    /// Effective permissions: role permissions plus direct grants.
    pub permissions: Vec<String>,
}
