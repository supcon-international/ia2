mod events;
mod routes;
mod sample;
mod state;

use axum::{
    Router,
    routing::{get, post},
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("server=debug,tower_http=info,info")),
        )
        .init();

    let state = AppState::new();

    let app = Router::new()
        .route("/health", get(routes::health))
        .route("/api/program", get(routes::program))
        .route("/api/check", post(routes::check))
        .route("/api/run", post(routes::run))
        .route("/api/stop", post(routes::stop))
        .route("/api/events", get(routes::events))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = "127.0.0.1:3001";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "server listening");
    axum::serve(listener, app).await?;
    Ok(())
}
