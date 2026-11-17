use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::Deserialize;

use crate::config::PortainerConfig;
use crate::error::AppError;

use super::dto::{ServerLogsDto, ServerStatusDto};

/// Thin client for the Portainer HTTP API, which proxies the Docker API of the
/// environment that runs the Minecraft container. Portainer is called with a
/// scoped API key, so the application never needs the Docker socket.
pub struct PortainerClient {
    http: reqwest::Client,
    config: PortainerConfig,
    /// Serializes start/stop/restart so two admin clicks cannot interleave.
    lifecycle: tokio::sync::Mutex<()>,
}

impl PortainerClient {
    pub fn new(config: Option<&PortainerConfig>) -> anyhow::Result<Option<Self>> {
        let Some(config) = config else {
            return Ok(None);
        };

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent(concat!("minecraft-mods-api/", env!("CARGO_PKG_VERSION")))
            .danger_accept_invalid_certs(!config.verify_tls)
            .build()
            .map_err(|err| anyhow::anyhow!("could not build the Portainer HTTP client: {err}"))?;

        Ok(Some(Self {
            http,
            config: config.clone(),
            lifecycle: tokio::sync::Mutex::new(()),
        }))
    }

    pub fn container(&self) -> &str {
        &self.config.container
    }

    /// Whether the container is running, used to gate mod file changes.
    pub async fn running(&self) -> Result<bool, AppError> {
        Ok(self.status().await?.running)
    }

    /// `GET /containers/{id}/json`.
    pub async fn status(&self) -> Result<ServerStatusDto, AppError> {
        let response = ensure_success(self.send(Method::GET, "json", &[]).await?)?;

        let inspect: InspectContainer = response
            .json()
            .await
            .map_err(|err| upstream(format!("could not decode the container status: {err}")))?;

        let state = inspect.state.unwrap_or_default();
        Ok(ServerStatusDto {
            container: inspect
                .name
                .map(|name| name.trim_start_matches('/').to_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| self.config.container.clone()),
            image: inspect.config.and_then(|config| config.image),
            state: state.status.unwrap_or_else(|| "unknown".to_owned()),
            running: state.running.unwrap_or(false),
            started_at: state.started_at,
            finished_at: state.finished_at,
            restart_count: state.restart_count.unwrap_or(0),
            health: state.health.and_then(|health| health.status),
        })
    }

    /// `POST /containers/{id}/start`.
    pub async fn start(&self) -> Result<ServerStatusDto, AppError> {
        let _guard = self.lifecycle.lock().await;

        ensure_success(self.send(Method::POST, "start", &[]).await?)?;

        self.status().await
    }

    /// `POST /containers/{id}/stop?t=<timeout>`.
    pub async fn stop(&self, timeout: u64) -> Result<ServerStatusDto, AppError> {
        let timeout = timeout.to_string();
        let _guard = self.lifecycle.lock().await;

        ensure_success(
            self.send(Method::POST, "stop", &[("t", timeout.as_str())])
                .await?,
        )?;

        self.status().await
    }

    /// `POST /containers/{id}/restart?t=<timeout>`.
    pub async fn restart(&self, timeout: u64) -> Result<ServerStatusDto, AppError> {
        let timeout = timeout.to_string();
        let _guard = self.lifecycle.lock().await;

        ensure_success(
            self.send(Method::POST, "restart", &[("t", timeout.as_str())])
                .await?,
        )?;

        self.status().await
    }

    /// `GET /containers/{id}/logs?stdout=1&stderr=1&tail=N`.
    pub async fn logs(&self, tail: u32, timestamps: bool) -> Result<ServerLogsDto, AppError> {
        let tail_param = tail.to_string();
        let timestamps_param = if timestamps { "1" } else { "0" };

        let response = ensure_success(
            self.send(
                Method::GET,
                "logs",
                &[
                    ("stdout", "1"),
                    ("stderr", "1"),
                    ("tail", tail_param.as_str()),
                    ("timestamps", timestamps_param),
                ],
            )
            .await?,
        )?;

        let bytes = response
            .bytes()
            .await
            .map_err(|err| upstream(format!("could not read the container logs: {err}")))?;

        Ok(ServerLogsDto {
            container: self.config.container.clone(),
            tail,
            timestamps,
            logs: decode_docker_stream(&bytes),
        })
    }

    fn url(&self, suffix: &str) -> String {
        format!(
            "{}/api/endpoints/{}/docker/containers/{}/{}",
            self.config.url, self.config.endpoint_id, self.config.container, suffix
        )
    }

    async fn send(
        &self,
        method: Method,
        suffix: &str,
        query: &[(&str, &str)],
    ) -> Result<reqwest::Response, AppError> {
        self.http
            .request(method.clone(), self.url(suffix))
            // Portainer access token; never logged.
            .header("X-API-Key", &self.config.api_key)
            .query(query)
            .send()
            .await
            .map_err(|err| {
                tracing::warn!(
                    container = %self.config.container,
                    method = %method,
                    error = %err,
                    "Portainer request failed"
                );
                upstream(format!("Portainer request failed: {err}"))
            })
    }
}

fn upstream(message: String) -> AppError {
    AppError::Upstream(message)
}

fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, AppError> {
    let status = response.status();
    // Docker answers 304 when start/stop is a no-op (the container is already
    // in the requested state); that is success from the caller's perspective.
    if status.is_success() || status == StatusCode::NOT_MODIFIED {
        return Ok(response);
    }

    match status {
        StatusCode::NOT_FOUND => Err(AppError::NotFound("Minecraft container")),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(AppError::Upstream(
            "Portainer rejected the API key".to_owned(),
        )),
        _ => {
            tracing::warn!(%status, "Portainer returned an error status");
            Err(AppError::Upstream(format!("Portainer returned {status}")))
        }
    }
}

/// Docker multiplexes stdout/stderr into frames unless the container has a TTY:
/// one stream byte, three zero bytes and a big-endian `u32` length. Portainer
/// passes that stream through unchanged, so it has to be decoded here.
pub fn decode_docker_stream(bytes: &[u8]) -> String {
    let mut logs = String::new();
    let mut rest = bytes;

    while rest.len() >= 8 {
        let looks_framed = matches!(rest[0], 0..=2) && rest[1..4] == [0, 0, 0];
        if !looks_framed {
            break;
        }

        let length = u32::from_be_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        let Some(frame) = rest.get(8..8 + length) else {
            break;
        };

        logs.push_str(&String::from_utf8_lossy(frame));
        rest = &rest[8 + length..];
    }

    // A TTY-enabled container (or an error page) yields a plain stream.
    if logs.is_empty() {
        return String::from_utf8_lossy(bytes).into_owned();
    }

    if !rest.is_empty() {
        logs.push_str(&String::from_utf8_lossy(rest));
    }

    logs
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectContainer {
    name: Option<String>,
    config: Option<InspectConfig>,
    state: Option<InspectState>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectConfig {
    image: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectState {
    status: Option<String>,
    running: Option<bool>,
    started_at: Option<String>,
    finished_at: Option<String>,
    restart_count: Option<u32>,
    health: Option<InspectHealth>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectHealth {
    status: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::decode_docker_stream;

    fn frame(stream: u8, payload: &str) -> Vec<u8> {
        let mut bytes = vec![stream, 0, 0, 0];
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(payload.as_bytes());
        bytes
    }

    #[test]
    fn decodes_a_multiplexed_docker_stream() {
        let mut bytes = frame(1, "[12:00:00] Starting minecraft server\n");
        bytes.extend(frame(2, "WARN: something odd\n"));

        assert_eq!(
            decode_docker_stream(&bytes),
            "[12:00:00] Starting minecraft server\nWARN: something odd\n"
        );
    }

    #[test]
    fn falls_back_to_plain_text_for_tty_containers() {
        assert_eq!(
            decode_docker_stream(b"plain log line\n"),
            "plain log line\n"
        );
    }

    #[test]
    fn keeps_a_truncated_trailing_frame() {
        let mut bytes = frame(1, "first\n");
        bytes.extend_from_slice(b"\x01\x00");

        assert_eq!(decode_docker_stream(&bytes), "first\n\x01\x00");
    }
}
