use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Current state of the Minecraft container, as reported by Docker.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServerStatusDto {
    /// Container name from `MC_CONTAINER`.
    pub container: String,
    /// Image the container was created from.
    pub image: Option<String>,
    /// Docker state: `created`, `running`, `exited`, `paused`, ...
    pub state: String,
    pub running: bool,
    /// RFC 3339 timestamp of the last start, or Docker's zero time.
    pub started_at: Option<String>,
    /// RFC 3339 timestamp of the last exit, or Docker's zero time.
    pub finished_at: Option<String>,
    pub restart_count: u32,
    /// Healthcheck result (`healthy`, `unhealthy`, `starting`), if configured.
    pub health: Option<String>,
}

/// The tail of the container log.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServerLogsDto {
    pub container: String,
    /// Number of lines requested (Docker's `tail`).
    pub tail: u32,
    /// Whether every line starts with its RFC 3339 timestamp.
    pub timestamps: bool,
    /// Decoded stdout+stderr, one entry per line.
    pub logs: String,
}

/// `?timeout=` for stop and restart.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LifecycleQuery {
    /// Seconds Docker waits for a graceful shutdown before `SIGKILL` (1-300, default 30).
    pub timeout: Option<u64>,
}

impl LifecycleQuery {
    pub fn timeout(&self) -> u64 {
        self.timeout.unwrap_or(30).clamp(1, 300)
    }
}

/// `?tail=&timestamps=` for the log endpoint.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LogsQuery {
    /// Number of trailing lines to return (1-5000, default 200).
    pub tail: Option<u32>,
    /// Prefix every line with its RFC 3339 timestamp.
    pub timestamps: Option<bool>,
}

impl LogsQuery {
    pub fn tail(&self) -> u32 {
        self.tail.unwrap_or(200).clamp(1, 5000)
    }

    pub fn timestamps(&self) -> bool {
        self.timestamps.unwrap_or(false)
    }
}
