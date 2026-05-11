mod error;
mod events;
mod routes;
mod sample;
mod state;

use axum::{
    Router,
    routing::{get, post},
};
use iomap_modbus::{DemoSlave, run_demo_slave};
use project::{ProjectStore, load_last_opened};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

const DEMO_MODBUS_ADDR: &str = "127.0.0.1:5502";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("server=debug,tower_http=info,info")),
        )
        .init();

    // Spin up the demo Modbus slave so users can wire a Modbus device at
    // 127.0.0.1:5502 without external hardware. The slave is shared with
    // AppState so the frontend can peek register/coil values.
    let demo_slave = DemoSlave::new();
    let state = AppState::new(demo_slave.clone());
    try_open_last_project(&state);

    let slave_for_task = demo_slave.clone();
    tokio::spawn(async move {
        let addr = DEMO_MODBUS_ADDR.parse().expect("valid socket addr");
        if let Err(e) = run_demo_slave(addr, slave_for_task).await {
            tracing::error!(%e, "demo modbus slave exited");
        }
    });

    let app = Router::new()
        .route("/health", get(routes::health))
        // Project lifecycle
        .route("/api/projects", get(routes::list_projects).post(routes::create_project))
        .route("/api/projects/open", post(routes::open_project))
        .route("/api/projects/close", post(routes::close_project))
        .route("/api/project", get(routes::project_tree))
        // Applications (POUs)
        .route("/api/applications", post(routes::create_application))
        .route(
            "/api/applications/{name}",
            get(routes::get_application)
                .put(routes::save_application)
                .delete(routes::delete_application),
        )
        .route(
            "/api/applications/{name}/variables",
            get(routes::application_variables),
        )
        // Devices
        .route("/api/devices", post(routes::create_device))
        .route(
            "/api/devices/{name}",
            get(routes::get_device)
                .put(routes::update_device)
                .delete(routes::delete_device),
        )
        // IO Mapping
        .route("/api/iomap", get(routes::get_iomap).put(routes::put_iomap))
        // Compile / runtime
        .route("/api/check", post(routes::check))
        .route("/api/run", post(routes::run))
        .route("/api/stop", post(routes::stop))
        .route("/api/events", get(routes::events))
        // LSP bridge — WebSocket-upgraded; the browser-side monaco-
        // languageclient connects here and talks LSP JSON-RPC to a
        // freshly-spawned ironplc LSP process.
        .route("/api/lsp", get(routes::lsp))
        // Internal: peek demo slave's memory for verification + UI display
        .route("/api/_demo/slave", get(routes::demo_slave))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = "127.0.0.1:3001";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn try_open_last_project(state: &AppState) {
    let Some(path) = load_last_opened() else {
        return;
    };
    match ProjectStore::open(path.clone()) {
        Ok(store) => {
            tracing::info!(path = %store.root().display(), "reopened last project");
            *state.project.lock().expect("project mutex") = Some(store);
        }
        Err(e) => tracing::warn!(?path, %e, "failed to reopen last project"),
    }
}
