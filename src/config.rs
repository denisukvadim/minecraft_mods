use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result};
use axum::http::HeaderValue;

/// Runtime configuration, loaded once from the process environment (`.env`).
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_ttl_minutes: i64,
    pub bind_addr: SocketAddr,
    pub cors_origins: Vec<HeaderValue>,
    pub allow_any_origin: bool,
    /// Directory that holds the mod files. Canonicalized at startup so every
    /// later path check compares against the real location.
    pub mods_dir: PathBuf,
    /// Upper bound for a single uploaded file, in bytes.
    pub mods_max_upload_bytes: u64,
    /// Optional bootstrap administrator. All three values must be set together.
    pub admin_email: Option<String>,
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let database_url = required("DATABASE_URL")?;

        let jwt_secret = required("JWT_SECRET")?;
        if jwt_secret.chars().count() < 32 {
            anyhow::bail!("JWT_SECRET must be at least 32 characters long");
        }

        let jwt_ttl_minutes = parse_or("JWT_TTL_MINUTES", 60_i64)?;
        if jwt_ttl_minutes <= 0 {
            anyhow::bail!("JWT_TTL_MINUTES must be a positive number");
        }

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_owned());
        let port: u16 = parse_or("PORT", 8080_u16)?;
        let bind_addr = format!("{host}:{port}")
            .parse()
            .with_context(|| format!("invalid HOST/PORT combination: {host}:{port}"))?;

        let raw_origins = env::var("CORS_ORIGINS").unwrap_or_else(|_| "*".to_owned());
        let mut cors_origins = Vec::new();
        let mut allow_any_origin = false;
        for origin in raw_origins
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
        {
            if origin == "*" {
                allow_any_origin = true;
                continue;
            }
            cors_origins.push(
                HeaderValue::from_str(origin).with_context(|| {
                    format!("CORS_ORIGINS contains an invalid origin: {origin}")
                })?,
            );
        }
        if !allow_any_origin && cors_origins.is_empty() {
            anyhow::bail!("CORS_ORIGINS must contain `*` or at least one origin");
        }

        let admin_email = optional("ADMIN_EMAIL")?;
        let admin_username = optional("ADMIN_USERNAME")?;
        let admin_password = optional("ADMIN_PASSWORD")?;
        let admin_values = [
            admin_email.is_some(),
            admin_username.is_some(),
            admin_password.is_some(),
        ];
        if admin_values.iter().any(|set| *set) && !admin_values.iter().all(|set| *set) {
            anyhow::bail!("ADMIN_EMAIL, ADMIN_USERNAME and ADMIN_PASSWORD must be set together");
        }

        let mods_dir = mods_dir()?;
        let mods_max_upload_bytes = parse_or("MODS_MAX_UPLOAD_BYTES", 128 * 1024 * 1024_u64)?;
        if mods_max_upload_bytes == 0 {
            anyhow::bail!("MODS_MAX_UPLOAD_BYTES must be greater than zero");
        }

        Ok(Self {
            database_url,
            jwt_secret,
            jwt_ttl_minutes,
            bind_addr,
            cors_origins,
            allow_any_origin,
            mods_dir,
            mods_max_upload_bytes,
            admin_email,
            admin_username,
            admin_password,
        })
    }
}

/// Resolves `MODS_DIR`, creating it when missing. The canonical path is what
/// handlers compare against, so a symlinked temp directory cannot redirect a
/// write outside the configured root.
fn mods_dir() -> Result<PathBuf> {
    let raw = env::var("MODS_DIR").unwrap_or_else(|_| "./data/mods".to_owned());
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("MODS_DIR must not be empty");
    }

    std::fs::create_dir_all(raw).with_context(|| format!("could not create MODS_DIR at {raw}"))?;

    let resolved = std::fs::canonicalize(raw)
        .with_context(|| format!("could not resolve MODS_DIR at {raw}"))?;
    if !resolved.is_dir() {
        anyhow::bail!("MODS_DIR must point to a directory: {}", resolved.display());
    }

    Ok(resolved)
}

fn required(key: &str) -> Result<String> {
    let value = env::var(key).with_context(|| format!("{key} must be set (see .env.example)"))?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        anyhow::bail!("{key} must not be empty");
    }
    Ok(value)
}

fn optional(key: &str) -> Result<Option<String>> {
    match env::var(key) {
        Ok(value) => {
            let value = value.trim().to_owned();
            Ok((!value.is_empty()).then_some(value))
        }
        Err(env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(anyhow::anyhow!("failed to read {key}: {err}")),
    }
}

fn parse_or<T>(key: &str, default: T) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match env::var(key) {
        Ok(raw) if !raw.trim().is_empty() => raw
            .trim()
            .parse::<T>()
            .map_err(|err| anyhow::anyhow!("{key} is invalid: {err}")),
        _ => Ok(default),
    }
}
