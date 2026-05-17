//! Headless edge runtime for IA2.
//!
//! Single binary, no IDE: load a project from disk → compile the named POU
//! → spawn the ironplc-bridge scan loop → expose a tiny HTTP server on a
//! local port so the IDE (via SSH port-forward) can stream `VarSnapshot`s
//! back for online debugging.
//!
//! Bind is `127.0.0.1` by default — remote access must go through SSH
//! port-forward, not direct exposure. There's intentionally no auth on
//! this server; the security perimeter is "only reachable via ssh".

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::State,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::stream::Stream;
use ironplc_bridge::{DeviceSpec, ProgramHandle, VarSnapshot};
use project::{ProjectStore, ProtocolConfig};
use serde::Serialize;
use tokio::signal;
use tower_http::cors::CorsLayer;
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND: &str = "127.0.0.1:13001";

/// Parsed CLI args. Manual parsing (no clap) — two flags, easy enough.
struct Args {
    project_dir: PathBuf,
    bind: SocketAddr,
}

fn parse_args() -> Result<Args> {
    let mut project_dir: Option<PathBuf> = None;
    let mut bind = DEFAULT_BIND.to_string();

    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--project-dir" => {
                project_dir = Some(PathBuf::from(
                    iter.next().context("--project-dir requires a value")?,
                ));
            }
            "--bind" => {
                bind = iter.next().context("--bind requires a value")?;
            }
            // Legacy flag from the pre-tasks-refactor builds. Accept but
            // ignore so existing systemd unit files keep launching.
            "--app" => {
                let _ = iter.next();
                tracing::warn!(
                    "--app is deprecated; the runtime now compiles the whole project. \
                     Update your systemd unit to drop this flag."
                );
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(anyhow!("unknown argument: {other}")),
        }
    }

    let project_dir = project_dir.context("--project-dir is required")?;
    let bind: SocketAddr = bind
        .parse()
        .with_context(|| format!("--bind '{bind}' is not a valid socket address"))?;

    Ok(Args { project_dir, bind })
}

fn print_help() {
    eprintln!(
        "ia2-runtime — headless edge runtime\n\n\
         USAGE:\n  \
         ia2-runtime --project-dir <path> [--bind <addr>]\n\n\
         FLAGS:\n  \
         --project-dir <path>   Path to the project directory (containing project.toml).\n  \
         --bind <addr>          Local socket for the monitor server (default {DEFAULT_BIND}).\n\n\
         The runtime exposes:\n  \
         GET  /health           Liveness check.\n  \
         GET  /status           Project + runtime metadata + last-known scan count.\n  \
         GET  /events           Server-Sent Events stream of VarSnapshot updates.\n  \
         POST /stop             Request graceful shutdown.\n\n\
         What runs is determined by the project's tasks.toml — every PROGRAM\n\
         instance declared there is bound to its task and scheduled.\n"
    );
}

/// Shared state for the HTTP handlers.
#[derive(Clone)]
struct AppState {
    project_name: String,
    /// PROGRAM instances actually scheduled, derived from tasks.toml.
    /// Reported by /status so attached IDEs / operators can see what's
    /// running without reading the project off-disk.
    program_instances: Vec<String>,
    devices: Vec<String>,
    /// Wall-clock when the runtime started accepting requests.
    start_time: Instant,
    /// Sender side of the runtime's snapshot fan-out. SSE handlers
    /// subscribe; the bridge subscriber task publishes.
    snapshot_tx: broadcast::Sender<VarSnapshot>,
    /// Most recent snapshot, kept so /status can return the latest scan
    /// count without subscribing.
    latest: Arc<Mutex<Option<VarSnapshot>>>,
    /// Flips true when /stop is hit; the main loop watches this and exits.
    shutdown: Arc<tokio::sync::Notify>,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    let args = parse_args()?;

    tracing::info!(
        project_dir = %args.project_dir.display(),
        bind = %args.bind,
        version = env!("CARGO_PKG_VERSION"),
        "ia2-runtime starting"
    );

    // ---- Load + compile the whole project ----
    let store = ProjectStore::open(args.project_dir.clone())
        .with_context(|| format!("opening project at {}", args.project_dir.display()))?;
    let project_name = store.name().to_string();
    let tasks = store
        .read_tasks()
        .context("reading tasks.toml")?
        .ok_or_else(|| {
            anyhow!(
                "tasks.toml missing from project — run the IDE's 'Migrate to tasks' \
                 once, or hand-author tasks.toml, then redeploy"
            )
        })?;
    let program_instances: Vec<String> =
        tasks.programs.iter().map(|p| p.instance.clone()).collect();
    let devices = store.list_devices().context("listing devices")?;
    let iomap = store.read_iomap().context("reading iomap")?;
    let device_names = devices.iter().map(|d| d.name.clone()).collect::<Vec<_>>();
    let device_specs: Vec<DeviceSpec> = devices
        .into_iter()
        .map(|d| DeviceSpec {
            name: d.name,
            config: d.config,
        })
        .collect();
    for spec in &device_specs {
        if let ProtocolConfig::Ethercat(cfg) = &spec.config {
            // iomap-ethercat treats nic="_sim" (or empty) as sim mode and
            // anything else as a real NIC name. We only warn for sim —
            // a real NIC means we'll attempt real fieldbus traffic, and
            // errors there will surface naturally as connect failures.
            if cfg.nic == "_sim" || cfg.nic.is_empty() {
                tracing::warn!(
                    device = %spec.name,
                    "ethercat device is in simulation mode (nic=\"_sim\") — no real fieldbus traffic"
                );
            } else {
                tracing::info!(
                    device = %spec.name,
                    nic = %cfg.nic,
                    "ethercat device configured for real bus"
                );
            }
        }
    }

    let container = ironplc_bridge::compile_project(&store).context("compiling project")?;
    tracing::info!(
        devices = device_specs.len(),
        mappings = iomap.mappings.len(),
        tasks = tasks.tasks.len(),
        programs = tasks.programs.len(),
        "compiled"
    );

    // ---- Spawn the bridge ----
    let handle: ProgramHandle = ironplc_bridge::spawn(container, device_specs, iomap.mappings);

    // Fan out the bridge's snapshots into a runtime-owned broadcast channel,
    // so we can keep the latest snapshot in shared state for /status and so
    // SSE clients can come and go without affecting the bridge subscriber.
    let (snapshot_tx, _) = broadcast::channel::<VarSnapshot>(128);
    let latest: Arc<Mutex<Option<VarSnapshot>>> = Arc::new(Mutex::new(None));
    let shutdown = Arc::new(tokio::sync::Notify::new());

    {
        let snapshot_tx = snapshot_tx.clone();
        let latest = latest.clone();
        let mut rx = handle.subscribe();
        tokio::spawn(async move {
            while let Ok(snap) = rx.recv().await {
                *latest.lock().expect("latest mutex") = Some(snap.clone());
                let _ = snapshot_tx.send(snap);
            }
        });
    }

    let state = AppState {
        project_name,
        program_instances,
        devices: device_names,
        start_time: Instant::now(),
        snapshot_tx,
        latest,
        shutdown: shutdown.clone(),
    };

    // ---- HTTP server ----
    // Permissive CORS: the only path to this socket on a real edge box is
    // through an SSH port-forward we (the dev) opened. The audience here
    // is "whatever dev tool is on the other end of the tunnel" — making
    // the IDE / curl / browser direct EventSource all work is the goal.
    let app = Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/events", get(events))
        .route("/stop", post(stop_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    tracing::info!(addr = %args.bind, "monitor server listening");

    // Serve until SIGTERM / SIGINT / POST /stop.
    let server = async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(wait_for_shutdown(shutdown))
            .await
    };
    let server_result = server.await;
    // Bridge is parked on its own std::thread + tokio current_thread runtime;
    // request_stop sets the AtomicBool the scan loop polls each round.
    tracing::info!("stop requested; draining bridge");
    handle.stop();
    // Give the scan loop a moment to flush; it's cooperative — no hard kill.
    tokio::time::sleep(Duration::from_millis(200)).await;
    tracing::info!("ia2-runtime exiting");
    server_result.map_err(Into::into)
}

/// Composite shutdown signal: any of SIGTERM, SIGINT, or POST /stop.
async fn wait_for_shutdown(shutdown: Arc<tokio::sync::Notify>) {
    #[cfg(unix)]
    let term = async {
        let mut sig = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        sig.recv().await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();

    tokio::select! {
        _ = signal::ctrl_c() => tracing::info!("received Ctrl+C"),
        _ = term => tracing::info!("received SIGTERM"),
        _ = shutdown.notified() => tracing::info!("received /stop"),
    }
}

fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                EnvFilter::new(
                    "ia2_runtime=info,ironplc_bridge=info,iomap_modbus=info,iomap_ethercat=info,info",
                )
            }),
        )
        .init();
}

// ============================================================
//  HTTP handlers
// ============================================================

#[derive(Serialize)]
struct Health {
    status: &'static str,
    uptime_secs: u64,
    scan_count: u64,
}

async fn health(State(state): State<AppState>) -> Json<Health> {
    let scan_count = state
        .latest
        .lock()
        .expect("latest")
        .as_ref()
        .map(|s| s.scan_count)
        .unwrap_or(0);
    Json(Health {
        status: "ok",
        uptime_secs: state.start_time.elapsed().as_secs(),
        scan_count,
    })
}

#[derive(Serialize)]
struct Status {
    version: &'static str,
    project: String,
    /// PROGRAM instances scheduled by the project's tasks.toml.
    program_instances: Vec<String>,
    devices: Vec<String>,
    uptime_secs: u64,
    scan_count: u64,
    last_snapshot: Option<VarSnapshot>,
}

async fn status(State(state): State<AppState>) -> Json<Status> {
    let last_snapshot = state.latest.lock().expect("latest").clone();
    let scan_count = last_snapshot.as_ref().map(|s| s.scan_count).unwrap_or(0);
    Json(Status {
        version: env!("CARGO_PKG_VERSION"),
        project: state.project_name.clone(),
        program_instances: state.program_instances.clone(),
        devices: state.devices.clone(),
        uptime_secs: state.start_time.elapsed().as_secs(),
        scan_count,
        last_snapshot,
    })
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // BroadcastStream surfaces both Ok(snap) and Err(Lagged); drop lagged
    // ticks rather than disconnect the client — the IDE just wants the
    // latest values, not lossless history.
    let rx = state.snapshot_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(snap) => Event::default().json_data(&snap).ok().map(Ok),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn stop_handler(State(state): State<AppState>) -> impl IntoResponse {
    state.shutdown.notify_waiters();
    Json(serde_json::json!({"ok": true}))
}
