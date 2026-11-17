use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::password;
use crate::config::Config;
use crate::rbac::service::PERMISSION_CATALOG;

/// Seeds the rule catalog, the two system roles and the optional bootstrap
/// administrator. Runs after migrations on every start.
pub async fn bootstrap(pool: &PgPool, config: &Config) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    for (code, description) in PERMISSION_CATALOG {
        sqlx::query(
            r#"
            INSERT INTO permissions (code, description)
            VALUES ($1, $2)
            ON CONFLICT (code) DO NOTHING
            "#,
        )
        .bind(*code)
        .bind(*description)
        .execute(&mut *tx)
        .await?;
    }

    let all_codes: Vec<String> = PERMISSION_CATALOG
        .iter()
        .map(|(code, _)| (*code).to_owned())
        .collect();

    // The admin role always regains the built-in catalog: deleting a rule by
    // accident would otherwise lock every administrator out until a manual DB fix.
    let admin_role_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO roles (name, description, is_system)
        VALUES ('admin', 'Full access', true)
        ON CONFLICT (name) DO UPDATE SET is_system = true
        RETURNING id
        "#,
    )
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO role_permissions (role_id, permission_id)
        SELECT $1, id FROM permissions WHERE code = ANY($2)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(admin_role_id)
    .bind(&all_codes)
    .execute(&mut *tx)
    .await?;

    // The default role is seeded only when it is created, so an administrator
    // can later take individual permissions away without the next restart
    // restoring them. The Mods operator baseline can manage files and the
    // container, and may onboard accounts with the same access.
    let new_user_role_id: Option<Uuid> = sqlx::query_scalar(
        r#"
        INSERT INTO roles (name, description, is_system)
        VALUES ('user', 'Default role for new accounts', true)
        ON CONFLICT (name) DO NOTHING
        RETURNING id
        "#,
    )
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(user_role_id) = new_user_role_id {
        sqlx::query(
            r#"
            INSERT INTO role_permissions (role_id, permission_id)
            SELECT $1, id FROM permissions
            WHERE code IN (
                'mods:read', 'mods:download', 'mods:upload', 'mods:delete',
                'server:manage', 'users:create'
            )
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(user_role_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    if let (Some(email), Some(username), Some(password)) = (
        &config.admin_email,
        &config.admin_username,
        &config.admin_password,
    ) {
        bootstrap_admin(pool, email, username, password).await?;
    }

    Ok(())
}

async fn bootstrap_admin(
    pool: &PgPool,
    email: &str,
    username: &str,
    password: &str,
) -> anyhow::Result<()> {
    let email = email.trim().to_lowercase();
    let username = username.trim().to_owned();

    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE email = $1)")
        .bind(&email)
        .fetch_one(pool)
        .await?;

    if exists {
        tracing::debug!(email = %email, "bootstrap admin already exists; skipping");
        return Ok(());
    }

    let password = password.to_owned();
    let password_hash = tokio::task::spawn_blocking(move || password::hash_password(&password))
        .await
        .context("admin password hashing task failed")??;

    let mut tx = pool.begin().await?;

    let user_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO users (email, username, password_hash)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(&email)
    .bind(&username)
    .bind(&password_hash)
    .fetch_one(&mut *tx)
    .await
    .with_context(|| format!("could not create bootstrap admin {email}"))?;

    sqlx::query(
        r#"
        INSERT INTO user_roles (user_id, role_id)
        SELECT $1, id FROM roles WHERE name = 'admin'
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    tracing::info!(email = %email, "bootstrap admin created");

    Ok(())
}
