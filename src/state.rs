use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;

/// Shared application state handed to every handler.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
}
