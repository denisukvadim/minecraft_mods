mod app;
mod auth;
mod config;
mod db;
mod error;
mod mods;
mod openapi;
mod pagination;
mod rbac;
mod seed;
mod server;
mod state;
mod users;
mod validation;

use std::sync::Arc;

use anyhow::Context;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    init_tracing();

    let config = Config::from_env()?;
    let bind_addr = config.bind_addr;

    let db = db::connect(&config)
        .await
        .context("database initialization failed")?;

    seed::bootstrap(&db, &config)
        .await
        .context("bootstrap seed failed")?;

    let portainer = server::client::PortainerClient::new(config.portainer.as_ref())
        .context("Portainer client initialization failed")?;
    if let Some(client) = &portainer {
        tracing::info!(
            container = client.container(),
            "Minecraft server control is enabled through Portainer"
        );
    }

    let state = AppState {
        db,
        config: Arc::new(config),
        portainer: portainer.map(Arc::new),
    };
    let router = app::build_router(state);

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("could not bind {bind_addr}"))?;

    tracing::info!(
        %bind_addr,
        "Minecraft Mods API is listening (Swagger UI: http://{bind_addr}/docs)"
    );

    axum::serve(listener, router)
        .await
        .context("server error")?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}
