mod edges;
mod error;
mod events;
mod routes;
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

/// Address the in-process demo Modbus TCP slave binds to. Override with
/// `DEMO_MODBUS_ADDR=host:port`; set to an empty string to disable the
/// slave entirely (useful when port 5502 is taken by something else).
const DEFAULT_DEMO_MODBUS_ADDR: &str = "127.0.0.1:5502";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("server=debug,tower_http=info,info")),
        )
        .init();

    // Spin up the demo Modbus slave so users can wire a Modbus device
    // against it without external hardware. The slave is shared with
    // AppState so the frontend can peek register/coil values.
    let demo_slave = DemoSlave::new();
    let demo_addr = std::env::var("DEMO_MODBUS_ADDR")
        .unwrap_or_else(|_| DEFAULT_DEMO_MODBUS_ADDR.into());
    let state = AppState::new(demo_slave.clone(), demo_addr.clone());
    try_open_last_project(&state);

    if !demo_addr.is_empty() {
        let slave_for_task = demo_slave.clone();
        let addr_for_task = demo_addr.clone();
        tokio::spawn(async move {
            match addr_for_task.parse() {
                Ok(addr) => {
                    if let Err(e) = run_demo_slave(addr, slave_for_task).await {
                        tracing::error!(%e, addr = %addr_for_task, "demo modbus slave exited");
                    }
                }
                Err(e) => tracing::error!(%e, addr = %addr_for_task, "DEMO_MODBUS_ADDR invalid"),
            }
        });
    } else {
        tracing::info!("demo modbus slave disabled (DEMO_MODBUS_ADDR=\"\")");
    }

    let app = Router::new()
        .route("/health", get(routes::health))
        .route("/api/health", get(routes::api_health))
        // Project lifecycle
        .route("/api/projects", get(routes::list_projects).post(routes::create_project))
        .route("/api/projects/open", post(routes::open_project))
        .route("/api/projects/close", post(routes::close_project))
        .route("/api/project", get(routes::project_tree))
        .route("/api/project/validate", post(routes::validate_project))
        .route("/api/project/variables", get(routes::project_variables))
        // Applications (POUs)
        .route("/api/applications", post(routes::create_application))
        .route(
            "/api/applications/folders",
            post(routes::create_application_folder),
        )
        .route(
            "/api/applications/folders/{*path}",
            axum::routing::delete(routes::delete_application_folder),
        )
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
            "/api/devices/folders",
            post(routes::create_device_folder),
        )
        .route(
            "/api/devices/folders/{*path}",
            axum::routing::delete(routes::delete_device_folder),
        )
        .route(
            "/api/devices/{name}",
            get(routes::get_device)
                .put(routes::update_device)
                .delete(routes::delete_device),
        )
        // Edges (deploy targets)
        .route("/api/edges", post(routes::create_edge))
        .route(
            "/api/edges/folders",
            post(routes::create_edge_folder),
        )
        .route(
            "/api/edges/folders/{*path}",
            axum::routing::delete(routes::delete_edge_folder),
        )
        .route(
            "/api/edges/{name}",
            get(routes::get_edge)
                .put(routes::update_edge)
                .delete(routes::delete_edge),
        )
        .route("/api/edges/{name}/probe", get(routes::probe_edge_route))
        .route("/api/edges/{name}/deploy", post(routes::deploy_edge_route))
        .route("/api/edges/{name}/attach", post(routes::attach_edge_route))
        .route("/api/edges/{name}/detach", post(routes::detach_edge_route))
        .route(
            "/api/edges/{name}/attachment",
            get(routes::attachment_status),
        )
        // IO Mapping
        .route("/api/iomap", get(routes::get_iomap).put(routes::put_iomap))
        // Tasks (project-level scheduling)
        .route("/api/tasks", get(routes::get_tasks).put(routes::put_tasks))
        .route("/api/project/migrate-tasks", post(routes::migrate_tasks))
        // Compile / runtime
        .route("/api/check", post(routes::check))
        .route("/api/run", post(routes::run))
        .route("/api/stop", post(routes::stop))
        .route("/api/events", get(routes::events))
        // Synchronous runtime queries — agent-friendly alternatives to SSE.
        .route("/api/runtime/snapshot", get(routes::runtime_snapshot))
        .route("/api/runtime/status", get(routes::runtime_status))
        .route(
            "/api/runtime/variables/{name}",
            post(routes::write_runtime_variable),
        )
        // LSP bridge — WebSocket-upgraded; the browser-side monaco-
        // languageclient connects here and talks LSP JSON-RPC to a
        // freshly-spawned ironplc LSP process.
        .route("/api/lsp", get(routes::lsp))
        // Internal: peek + poke the in-process demo Modbus slave.
        .route("/api/_demo/slave", get(routes::demo_slave))
        .route(
            "/api/_demo/slave/{kind}/{addr}",
            axum::routing::put(routes::poke_demo_slave),
        )
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
