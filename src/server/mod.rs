//! Control of the Minecraft container through the Portainer HTTP API.
//!
//! Portainer already runs next to Docker and proxies the Docker API, so the
//! application can stop, start, restart and inspect the server without mounting
//! `/var/run/docker.sock` into its own container.

pub mod client;
pub mod dto;
pub mod routes;

use crate::error::AppError;

use client::PortainerClient;

/// Refuses mod file changes while the Minecraft server is running: the loader
/// reads jars only at boot, and deleting a file that a running server may still
/// map is a needless risk.
///
/// Without a Portainer integration there is no state to check, so callers
/// proceed. A Portainer failure is propagated: failing closed is safer than
/// changing files behind a server whose state is unknown.
pub async fn ensure_server_stopped(portainer: Option<&PortainerClient>) -> Result<(), AppError> {
    let Some(client) = portainer else {
        return Ok(());
    };

    if client.running().await? {
        return Err(AppError::Conflict(
            "the Minecraft server is running; stop it before uploading or deleting mod files"
                .to_owned(),
        ));
    }

    Ok(())
}
