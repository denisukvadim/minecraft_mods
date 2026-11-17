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
    /// Optional Portainer integration: when absent, the `/server/*` endpoints
    /// answer 503 instead of failing at startup.
    pub portainer: Option<PortainerConfig>,
}

/// Connection settings for the Portainer instance that owns the Minecraft
/// container. Portainer proxies the Docker API, so no Docker socket is needed.
#[derive(Debug, Clone)]
pub struct PortainerConfig {
    /// Base URL without the trailing slash, e.g. `https://portainer.local:9443`.
    pub url: String,
    pub api_key: String,
    /// Portainer environment (endpoint) that hosts the container.
    pub endpoint_id: u32,
    /// Name of the Minecraft container.
    pub container: String,
    /// Set to `false` only for a self-signed certificate you cannot add as a CA.
    pub verify_tls: bool,
    pub timeout_secs: u64,
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

        let portainer = portainer()?;

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
            portainer,
        })
    }
}

/// Reads the optional Portainer settings. The integration is either fully
/// configured or absent: half a configuration is a startup error, not a
/// silently disabled feature.
fn portainer() -> Result<Option<PortainerConfig>> {
    let url = optional("PORTAINER_URL")?;
    let api_key = optional("PORTAINER_API_KEY")?;

    let (url, api_key) = match (url, api_key) {
        (None, None) => return Ok(None),
        (Some(url), Some(api_key)) => (url, api_key),
        _ => anyhow::bail!("PORTAINER_URL and PORTAINER_API_KEY must be set together"),
    };

    let url = url.trim_end_matches('/').to_owned();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        anyhow::bail!("PORTAINER_URL must start with http:// or https://");
    }

    let endpoint_id = parse_or("PORTAINER_ENDPOINT_ID", 1_u32)?;
    if endpoint_id == 0 {
        anyhow::bail!("PORTAINER_ENDPOINT_ID must be greater than zero");
    }

    let container = env::var("MC_CONTAINER").unwrap_or_else(|_| "minecraft".to_owned());
    let container = container.trim().to_owned();
    if !is_container_name(&container) {
        anyhow::bail!(
            "MC_CONTAINER must start with a letter or digit and contain only letters, digits, `_`, `.` and `-`"
        );
    }

    let verify_tls = parse_bool_or("PORTAINER_VERIFY_TLS", true)?;
    let timeout_secs = parse_or("PORTAINER_TIMEOUT_SECS", 15_u64)?;
    if timeout_secs == 0 {
        anyhow::bail!("PORTAINER_TIMEOUT_SECS must be greater than zero");
    }

    Ok(Some(PortainerConfig {
        url,
        api_key,
        endpoint_id,
        container,
        verify_tls,
        timeout_secs,
    }))
}

/// Docker container names are restricted at creation time, but the value is
/// interpolated into a URL path, so it is validated again here.
fn is_container_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
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

fn parse_bool_or(key: &str, default: bool) -> Result<bool> {
    match env::var(key) {
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "" => Ok(default),
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => anyhow::bail!("{key} must be a boolean, got `{other}`"),
        },
        Err(env::VarError::NotPresent) => Ok(default),
        Err(err) => Err(anyhow::anyhow!("failed to read {key}: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::is_container_name;

    #[test]
    fn accepts_docker_container_names() {
        for name in ["minecraft", "mc_1", "mc-server.external", "MC01"] {
            assert!(is_container_name(name), "{name} should be accepted");
        }
    }

    #[test]
    fn rejects_values_that_could_break_the_url() {
        for name in [
            "",
            "-mc",
            "mc/server",
            "mc server",
            "mc?x=1",
            "mc#frag",
            "..",
            "mc%2F",
        ] {
            assert!(!is_container_name(name), "{name} should be rejected");
        }
    }
}
