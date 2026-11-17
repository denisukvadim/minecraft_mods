use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;
use crate::server::client::PortainerClient;

/// Shared application state handed to every handler.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    /// `None` when Portainer is not configured; `/server/*` then answers 503.
    pub portainer: Option<Arc<PortainerClient>>,
}
