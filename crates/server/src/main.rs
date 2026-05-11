use std::sync::OnceLock;
use std::time::Instant;

use axum::{Json, Router, routing::get};
use serde::{Deserialize, Serialize};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::EnvFilter;
use ts_rs::TS;

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthStatus {
    pub status: String,
    pub uptime_secs: u64,
}

static START: OnceLock<Instant> = OnceLock::new();

async fn health() -> Json<HealthStatus> {
    let elapsed = START.get().map(|t| t.elapsed().as_secs()).unwrap_or(0);
    Json(HealthStatus {
        status: "ok".into(),
        uptime_secs: elapsed,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("server=debug,tower_http=debug,info")),
        )
        .init();

    START.set(Instant::now()).ok();

    let app = Router::new()
        .route("/health", get(health))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr = "127.0.0.1:3001";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "server listening");
    axum::serve(listener, app).await?;
    Ok(())
}
