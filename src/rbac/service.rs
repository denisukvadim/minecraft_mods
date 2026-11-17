use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::validation;

use super::dto::{AccessResponse, PermissionDto, RoleDto, RoleSummary};

/// Seeded on every boot so a fresh database is usable and the `admin` role
/// always regains the full set (otherwise a mistaken edit could lock everyone out).
pub const PERMISSION_CATALOG: &[(&str, &str)] = &[
    ("mods:read", "List the mod files"),
    ("mods:download", "Download mod files"),
    ("mods:upload", "Upload mod files"),
    ("mods:delete", "Delete mod files"),
    ("users:read", "View users"),
    (
        "users:create",
        "Create users with access not exceeding your own",
    ),
    ("roles:read", "View roles"),
    ("permissions:read", "View the permission catalog"),
    (
        "server:manage",
        "Inspect, start, stop and restart the Minecraft server",
    ),
];

#[derive(Debug, sqlx::FromRow)]
pub struct RoleRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub is_system: bool,
    pub created_at: DateTime<Utc>,
}

/// Checks the union of role permissions and direct user grants.
pub async fn require_permission(pool: &PgPool, user_id: Uuid, code: &str) -> Result<(), AppError> {
    let granted: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM permissions p
            JOIN role_permissions rp ON rp.permission_id = p.id
            JOIN user_roles ur ON ur.role_id = rp.role_id
            WHERE ur.user_id = $1 AND p.code = $2
            UNION ALL
            SELECT 1
            FROM permissions p
            JOIN user_permissions up ON up.permission_id = p.id
            WHERE up.user_id = $1 AND p.code = $2
        )
        "#,
    )
    .bind(user_id)
    .bind(code)
    .fetch_one(pool)
    .await?;

    if granted {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

/// Whether the user carries the system `admin` role.
pub async fn is_admin(pool: &PgPool, user_id: Uuid) -> Result<bool, AppError> {
    let is_admin: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM user_roles ur
            JOIN roles r ON r.id = ur.role_id
            WHERE ur.user_id = $1 AND r.name = 'admin'
        )
        "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    Ok(is_admin)
}

/// Management endpoints (updating or deleting users, roles, permissions, and
/// assigning access) are restricted to the system `admin` role. This is
/// deliberately an identity check instead of a permission: a role that could
/// edit permissions could grant itself anything. Creating a user is the one
/// delegated exception (`users:create`), and it copies the creator's access.
pub async fn require_admin(pool: &PgPool, user_id: Uuid) -> Result<(), AppError> {
    if is_admin(pool, user_id).await? {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

pub async fn role_names_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT r.name
        FROM roles r
        JOIN user_roles ur ON ur.role_id = r.id
        WHERE ur.user_id = $1
        ORDER BY r.name
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn roles_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<RoleSummary>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT r.id, r.name
        FROM roles r
        JOIN user_roles ur ON ur.role_id = r.id
        WHERE ur.user_id = $1
        ORDER BY r.name
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn direct_permissions_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<PermissionDto>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT p.id, p.code, p.description, p.created_at
        FROM permissions p
        JOIN user_permissions up ON up.permission_id = p.id
        WHERE up.user_id = $1
        ORDER BY p.code
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn permission_codes_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT p.code
        FROM permissions p
        JOIN role_permissions rp ON rp.permission_id = p.id
        JOIN user_roles ur ON ur.role_id = rp.role_id
        WHERE ur.user_id = $1
        UNION
        SELECT p.code
        FROM permissions p
        JOIN user_permissions up ON up.permission_id = p.id
        WHERE up.user_id = $1
        ORDER BY 1
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn access_for_user(pool: &PgPool, user_id: Uuid) -> Result<AccessResponse, AppError> {
    Ok(AccessResponse {
        user_id,
        roles: roles_for_user(pool, user_id).await?,
        direct_permissions: direct_permissions_for_user(pool, user_id).await?,
        effective_permissions: permission_codes_for_user(pool, user_id).await?,
    })
}

pub async fn find_permission(
    pool: &PgPool,
    permission_id: Uuid,
) -> Result<PermissionDto, AppError> {
    sqlx::query_as("SELECT id, code, description, created_at FROM permissions WHERE id = $1")
        .bind(permission_id)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::NotFound("permission"))
}

pub async fn load_role(pool: &PgPool, role_id: Uuid) -> Result<RoleDto, AppError> {
    let row: RoleRow = sqlx::query_as(
        "SELECT id, name, description, is_system, created_at FROM roles WHERE id = $1",
    )
    .bind(role_id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound("role"))?;

    let permissions: Vec<PermissionDto> = sqlx::query_as(
        r#"
        SELECT p.id, p.code, p.description, p.created_at
        FROM permissions p
        JOIN role_permissions rp ON rp.permission_id = p.id
        WHERE rp.role_id = $1
        ORDER BY p.code
        "#,
    )
    .bind(role_id)
    .fetch_all(pool)
    .await?;

    Ok(RoleDto {
        id: row.id,
        name: row.name,
        description: row.description,
        is_system: row.is_system,
        created_at: row.created_at,
        permissions,
    })
}

pub async fn load_roles(pool: &PgPool) -> Result<Vec<RoleDto>, AppError> {
    let role_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM roles ORDER BY name")
        .fetch_all(pool)
        .await?;

    let mut roles = Vec::with_capacity(role_ids.len());
    for role_id in role_ids {
        roles.push(load_role(pool, role_id).await?);
    }
    Ok(roles)
}

/// Replaces the permission set of a role inside an existing transaction.
/// Unknown codes reject the whole operation, so a typo cannot silently
/// produce a role with fewer rules than intended.
pub async fn replace_permissions_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    role_id: Uuid,
    codes: &[String],
) -> Result<(), AppError> {
    let mut unique: Vec<String> = codes
        .iter()
        .map(|code| code.trim().to_owned())
        .filter(|code| !code.is_empty())
        .collect();
    unique.sort_unstable();
    unique.dedup();

    for code in &unique {
        validation::permission_code(code)?;
    }

    if !unique.is_empty() {
        let found: Vec<String> =
            sqlx::query_scalar("SELECT code FROM permissions WHERE code = ANY($1)")
                .bind(&unique)
                .fetch_all(&mut **tx)
                .await?;

        if found.len() != unique.len() {
            let found_set: HashSet<&str> = found.iter().map(String::as_str).collect();
            let missing: Vec<&str> = unique
                .iter()
                .map(String::as_str)
                .filter(|code| !found_set.contains(*code))
                .collect();
            return Err(AppError::Validation(format!(
                "unknown permission codes: {}",
                missing.join(", ")
            )));
        }
    }

    sqlx::query("DELETE FROM role_permissions WHERE role_id = $1")
        .bind(role_id)
        .execute(&mut **tx)
        .await?;

    sqlx::query(
        r#"
        INSERT INTO role_permissions (role_id, permission_id)
        SELECT $1, id FROM permissions WHERE code = ANY($2)
        "#,
    )
    .bind(role_id)
    .bind(&unique)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn replace_role_permissions(
    pool: &PgPool,
    role_id: Uuid,
    codes: &[String],
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    replace_permissions_in_tx(&mut tx, role_id, codes).await?;
    tx.commit().await?;
    Ok(())
}
