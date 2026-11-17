use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::rbac;

use super::dto::{UserDetail, UserSummary};

pub async fn load_user_summary(pool: &PgPool, user_id: Uuid) -> Result<UserSummary, AppError> {
    sqlx::query_as::<_, UserSummary>(
        "SELECT id, email, username, is_active, created_at FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound("user"))
}

pub async fn load_user_detail(pool: &PgPool, user_id: Uuid) -> Result<UserDetail, AppError> {
    let user = load_user_summary(pool, user_id).await?;
    let access = rbac::service::access_for_user(pool, user_id).await?;

    Ok(UserDetail {
        id: user.id,
        email: user.email,
        username: user.username,
        is_active: user.is_active,
        created_at: user.created_at,
        roles: access.roles,
        direct_permissions: access.direct_permissions,
        effective_permissions: access.effective_permissions,
    })
}
