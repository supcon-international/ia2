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

use std::collections::VecDeque;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use futures_util::stream::Stream;
use ironplc_bridge::{
    DeviceReport, DeviceSpec, ProgramHandle, RuntimeMode, RuntimeWriteError, VarSnapshot,
};
use project::{ProjectStore, ProtocolConfig};
use serde::Serialize;
use tokio::signal;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND: &str = "127.0.0.1:13001";

/// Upper bound on the graceful bridge drain (stop scan loop -> failsafe ->
/// join the EtherCAT cyclic thread). Comfortably under systemd's
/// `TimeoutStopSec=10s` so a clean exit always wins the race against
/// SIGKILL; if a wedged bus blows this budget we log and exit anyway,
/// falling back to the drive's own watchdog.
const BRIDGE_DRAIN_TIMEOUT: Duration = Duration::from_secs(7);

// ============================================================
//  Log capture — tee tracing output into a ring buffer + a live
//  broadcast so the monitor server can surface it (GET /logs and
//  /logs/stream). This is what makes edge-side truth (EtherCAT
//  discovery, bus health, device connect failures) visible to the
//  IDE / CLI over the SSH tunnel, instead of being trapped in
//  journald on the box.
// ============================================================

#[derive(Clone)]
struct LogCapture {
    buf: Arc<Mutex<VecDeque<String>>>,
    tx: broadcast::Sender<String>,
    cap: usize,
}

impl LogCapture {
    fn new(cap: usize) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            buf: Arc::new(Mutex::new(VecDeque::new())),
            tx,
            cap,
        }
    }

    fn push_line(&self, line: String) {
        {
            let mut b = self.buf.lock().expect("log buffer");
            while b.len() >= self.cap {
                b.pop_front();
            }
            b.push_back(line.clone());
        }
        // No subscribers is fine — ignore the send error.
        let _ = self.tx.send(line);
    }

    /// Most recent `n` captured lines, oldest-first.
    fn tail(&self, n: usize) -> Vec<String> {
        let b = self.buf.lock().expect("log buffer");
        let start = b.len().saturating_sub(n);
        b.iter().skip(start).cloned().collect()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
    type Writer = CaptureWriter;
    fn make_writer(&'a self) -> Self::Writer {
        CaptureWriter {
            cap: self.clone(),
            buf: Vec::new(),
        }
    }
}

/// One formatted event's bytes accumulate here; on Drop (end of that
/// event's write) we split into lines and push them to the capture.
/// `fmt` formats an event into one buffer then writes it once, so a
/// fresh writer per event maps cleanly to "one (or few) line(s)".
struct CaptureWriter {
    cap: LogCapture,
    buf: Vec<u8>,
}

impl std::io::Write for CaptureWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for CaptureWriter {
    fn drop(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let s = String::from_utf8_lossy(&self.buf);
        for line in s.split('\n') {
            let trimmed = line.trim_end();
            if !trimmed.is_empty() {
                self.cap.push_line(trimmed.to_string());
            }
        }
    }
}

/// Parsed CLI args. Manual parsing (no clap) — three flags, still simple.
struct Args {
    project_dir: PathBuf,
    bind: SocketAddr,
    /// Where to load/save RETAIN variable values. Defaults to
    /// `<project_dir>/../state/retain.json`, matching how
    /// `infra/install.sh` lays out a typical edge deployment
    /// (`/opt/ia2/state/retain.json` alongside `/opt/ia2/current/`).
    state_dir: PathBuf,
}

fn parse_args() -> Result<Args> {
    let mut project_dir: Option<PathBuf> = None;
    let mut bind = DEFAULT_BIND.to_string();
    let mut state_dir: Option<PathBuf> = None;

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
            "--state-dir" => {
                state_dir = Some(PathBuf::from(
                    iter.next().context("--state-dir requires a value")?,
                ));
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
    // Default: the sibling `state/` directory of the project, so a
    // project at `/opt/ia2/current/project` gets state at
    // `/opt/ia2/state/`. Survives `current` symlink rotations on
    // redeploy because the path is anchored to the install root, not
    // the version dir.
    let state_dir = state_dir.unwrap_or_else(|| {
        project_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|root| root.join("state"))
            .unwrap_or_else(|| project_dir.join("state"))
    });

    Ok(Args {
        project_dir,
        bind,
        state_dir,
    })
}

fn print_help() {
    eprintln!(
        "ia2-runtime — headless edge runtime\n\n\
         USAGE:\n  \
         ia2-runtime --project-dir <path> [--bind <addr>] [--state-dir <path>]\n\n\
         FLAGS:\n  \
         --project-dir <path>   Path to the project directory (containing project.toml).\n  \
         --bind <addr>          Local socket for the monitor server (default {DEFAULT_BIND}).\n  \
         --state-dir <path>     Where to persist RETAIN variables (default: sibling 'state/' of\n                         \
         the install root). Survives version swaps; safe to back up.\n\n\
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
    /// Captured tracing output (ring buffer + live broadcast) backing
    /// GET /logs and /logs/stream.
    logs: LogCapture,
    /// Handle to the running scan loop — used by /discover to read the
    /// per-device connect reports (connected/failed + EtherCAT topology).
    handle: ProgramHandle,
}

#[tokio::main]
async fn main() -> Result<()> {
    let log_capture = init_logging();
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

    // Multi-PROGRAM guard — see the IDE-side `/api/run` handler for
    // the full explanation. Same reason here on the edge: ironplc's
    // codegen only honors the first PROGRAM declaration.
    if tasks.programs.len() > 1 {
        let names = tasks
            .programs
            .iter()
            .map(|p| p.program.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "tasks.toml schedules {} PROGRAMs ({}) but the runtime can only execute \
             one PROGRAM per process (ironplc codegen limitation). Reduce tasks.toml \
             to one PROGRAM and redeploy.",
            tasks.programs.len(),
            names,
        );
    }

    let (container, metadata) =
        ironplc_bridge::compile_project_full(&store).context("compiling project")?;
    tracing::info!(
        devices = device_specs.len(),
        mappings = iomap.mappings.len(),
        tasks = tasks.tasks.len(),
        programs = tasks.programs.len(),
        retain_vars = metadata.retain_vars.len(),
        "compiled"
    );

    // Source the scan period from the (only) program's bound task,
    // falling back to the bridge default if the bind chain is
    // incomplete. See `crates/ironplc-bridge/src/runtime.rs` for why
    // we throttle in the bridge rather than via the VM scheduler.
    let scan_interval_ms = tasks
        .programs
        .first()
        .and_then(|p| tasks.tasks.iter().find(|t| t.name == p.task))
        .map(|t| t.interval_ms as u64)
        .unwrap_or(ironplc_bridge::DEFAULT_SCAN_INTERVAL_MS);

    // RETAIN state file lives under the configured state dir. The
    // bridge handles missing-file / bad-content gracefully, so we
    // don't pre-create anything here. Skip the path entirely if the
    // program declares no RETAIN vars — no file means no future
    // confusion about "what's in this state.json".
    let state_path = if metadata.retain_vars.is_empty() {
        None
    } else {
        let p = args.state_dir.join("retain.json");
        tracing::info!(state_path = %p.display(), "RETAIN state file");
        Some(p)
    };

    // ---- Spawn the bridge ----
    let handle: ProgramHandle = ironplc_bridge::spawn_with_options(
        container,
        device_specs,
        iomap.mappings,
        ironplc_bridge::SpawnOptions {
            scan_interval_ms,
            retain_vars: metadata.retain_vars,
            state_path,
        },
    );

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
        logs: log_capture,
        handle: handle.clone(),
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
        .route("/logs", get(logs))
        .route("/logs/stream", get(logs_stream))
        .route("/discover", get(discover))
        .route("/system", get(system))
        .route("/pause", post(rt_pause))
        .route("/resume", post(rt_resume))
        .route("/step", post(rt_step))
        .route("/write", post(rt_write))
        .route("/force", post(rt_force))
        .route("/unforce", post(rt_unforce))
        .route("/stop", post(stop_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    tracing::info!(addr = %args.bind, "monitor server listening");

    // Run the HTTP server as a background task. We deliberately do NOT gate
    // shutdown on it via `with_graceful_shutdown`: the long-lived SSE
    // streams (/events, /logs/stream) never end on their own, so graceful
    // shutdown would block on them — which previously stalled the whole
    // process past systemd's TimeoutStopSec and got us SIGKILLed *before*
    // the failsafe ran (so a connected servo only de-energized via its own
    // bus watchdog). Instead we own the shutdown sequence below: drive the
    // safety-critical bridge drain first, then drop the server.
    let server_task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(%e, "monitor server error");
        }
    });

    // Block until SIGTERM / SIGINT / POST /stop.
    wait_for_shutdown(shutdown).await;

    // ---- Safety-critical shutdown ----
    // Stop the scan loop, drive every device to failsafe (zero outputs —
    // for a servo that's controlword=0 / Disable Voltage), and JOIN the
    // EtherCAT cyclic thread so that zeroed frame is actually on the wire
    // before we exit. `handle.shutdown()` blocks until the scan thread has
    // done all of that. Bounded so a wedged bus can't hold us past the
    // service supervisor's kill timeout.
    tracing::info!(
        "shutdown signalled; draining bridge (stop scan loop -> failsafe -> join fieldbus)"
    );
    match tokio::time::timeout(BRIDGE_DRAIN_TIMEOUT, handle.shutdown()).await {
        Ok(()) => tracing::info!("bridge drained; outputs in failsafe"),
        Err(_) => tracing::error!(
            timeout_s = BRIDGE_DRAIN_TIMEOUT.as_secs(),
            "bridge drain timed out; exiting anyway (outputs fall back to the drive watchdog)"
        ),
    }

    // The server has done its job; stop accepting and drop any in-flight
    // SSE connections. Clients on the SSH tunnel handle the disconnect.
    server_task.abort();
    tracing::info!("ia2-runtime exiting");
    Ok(())
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

/// Set up tracing with two fmt layers under one env-filter: the usual
/// stdout/journald sink (kept untouched) plus a capture layer (ANSI
/// stripped) that tees into the returned `LogCapture` for /logs.
fn init_logging() -> LogCapture {
    use tracing_subscriber::prelude::*;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "ia2_runtime=info,ironplc_bridge=info,iomap_modbus=info,iomap_ethercat=info,info",
        )
    });
    let capture = LogCapture::new(2000);
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(capture.clone()),
        )
        .init();
    capture
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

/// One pinned (forced) variable in the runtime's debug state.
#[derive(Serialize)]
struct ForceEntry {
    name: String,
    value: i32,
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
    /// Scan-loop execution mode (running / paused / step) — debug state
    /// so an attached IDE can show "paused" / step progress.
    mode: RuntimeMode,
    /// Currently-forced variables (name → pinned value).
    forces: Vec<ForceEntry>,
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
        mode: state.handle.mode(),
        forces: state
            .handle
            .forces()
            .into_iter()
            .map(|(name, value)| ForceEntry { name, value })
            .collect(),
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

#[derive(serde::Deserialize)]
struct LogQuery {
    /// How many recent lines to return (default 200).
    tail: Option<usize>,
}

#[derive(Serialize)]
struct LogsResponse {
    lines: Vec<String>,
}

/// One-shot: the most recent `tail` (default 200) captured log lines.
/// This is what `cs edge logs` pulls over the SSH tunnel — surfacing
/// EtherCAT discovery / bus-health / connect errors that previously
/// only existed in journald on the edge.
async fn logs(State(state): State<AppState>, Query(q): Query<LogQuery>) -> Json<LogsResponse> {
    let n = q.tail.unwrap_or(200);
    Json(LogsResponse {
        lines: state.logs.tail(n),
    })
}

/// SSE: live log lines as they're emitted. No backlog — pair with
/// GET /logs for history (same split as /events vs /status).
async fn logs_stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.logs.tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(line) => Some(Ok(Event::default().data(line))),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Per-device connect reports + discovered EtherCAT topology. Powers
/// `cs edge scan` — the IDE authors PDO maps against the real bus.
async fn discover(State(state): State<AppState>) -> Json<Vec<DeviceReport>> {
    Json(state.handle.device_reports())
}

#[derive(Serialize)]
struct Nic {
    name: String,
    mac: String,
    /// `up` / `down` / `unknown` from the kernel.
    operstate: String,
    /// Whether a link partner is detected (cable in + powered).
    carrier: bool,
}

#[derive(Serialize)]
struct SystemInfo {
    arch: String,
    os: String,
    /// Network interfaces — pick one for an EtherCAT device's `nic`.
    nics: Vec<Nic>,
    /// Serial device paths — pick one for a Modbus RTU device.
    serial_ports: Vec<String>,
}

/// Enumerate the edge's interfaces / serial ports / arch so the IDE can
/// author device configs against real facts (which NIC has carrier for
/// EtherCAT, which /dev/tty* exists for Modbus RTU) instead of guessing.
/// Non-privileged: reads /sys/class/net and lists /dev.
fn collect_system_info() -> SystemInfo {
    let mut nics = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let p = e.path();
            let read = |f: &str| std::fs::read_to_string(p.join(f)).unwrap_or_default();
            nics.push(Nic {
                mac: read("address").trim().to_string(),
                operstate: read("operstate").trim().to_string(),
                carrier: read("carrier").trim() == "1",
                name,
            });
        }
    }
    nics.sort_by(|a, b| a.name.cmp(&b.name));

    let mut serial_ports = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev") {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            // USB-serial adapters (the common Modbus RTU case) + macOS
            // callout devices. `ttyS*` (builtin UARTs) are skipped — they
            // exist as nodes even with no hardware and just add noise.
            if n.starts_with("ttyUSB") || n.starts_with("ttyACM") || n.starts_with("cu.") {
                serial_ports.push(format!("/dev/{n}"));
            }
        }
    }
    serial_ports.sort();

    SystemInfo {
        arch: std::env::consts::ARCH.to_string(),
        os: std::env::consts::OS.to_string(),
        nics,
        serial_ports,
    }
}

/// Edge interfaces / serial ports / arch — powers `cs edge system`.
async fn system() -> Json<SystemInfo> {
    Json(collect_system_info())
}

// ============================================================
//  Online debug control — pause / resume / step / write / force.
//  Mirrors the IDE-side /api/runtime/* surface so an attached edge is a
//  full debug target, not just observable. NOTE: write/force can drive
//  real outputs on a connected bus — same power (and responsibility) as
//  forcing a variable on the local runtime; the scan loop's failsafe
//  still zeroes outputs on stop.
// ============================================================

#[derive(Serialize)]
struct ModeResp {
    mode: RuntimeMode,
}

async fn rt_pause(State(state): State<AppState>) -> Json<ModeResp> {
    state.handle.pause();
    Json(ModeResp {
        mode: state.handle.mode(),
    })
}

async fn rt_resume(State(state): State<AppState>) -> Json<ModeResp> {
    state.handle.resume();
    Json(ModeResp {
        mode: state.handle.mode(),
    })
}

#[derive(serde::Deserialize)]
struct StepReq {
    cycles: u32,
}

async fn rt_step(State(state): State<AppState>, Json(req): Json<StepReq>) -> Json<ModeResp> {
    state.handle.step(req.cycles);
    Json(ModeResp {
        mode: state.handle.mode(),
    })
}

#[derive(serde::Deserialize)]
struct VarWriteReq {
    name: String,
    /// Bit-packed value — the caller resolves the variable's type, same
    /// as the IDE-side runtime write/force surface.
    value: i32,
}

#[derive(serde::Deserialize)]
struct VarNameReq {
    name: String,
}

fn write_err(e: RuntimeWriteError) -> (StatusCode, String) {
    match e {
        RuntimeWriteError::UnknownVariable(n) => {
            (StatusCode::NOT_FOUND, format!("unknown variable '{n}'"))
        }
        RuntimeWriteError::Disconnected => {
            (StatusCode::CONFLICT, "scan loop has stopped".to_string())
        }
        RuntimeWriteError::Vm(e) => (StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn rt_write(
    State(state): State<AppState>,
    Json(req): Json<VarWriteReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let v = state
        .handle
        .write_variable(&req.name, req.value)
        .await
        .map_err(write_err)?;
    Ok(Json(
        serde_json::json!({ "ok": true, "name": req.name, "value": v }),
    ))
}

async fn rt_force(
    State(state): State<AppState>,
    Json(req): Json<VarWriteReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let v = state
        .handle
        .force_variable(&req.name, req.value)
        .await
        .map_err(write_err)?;
    Ok(Json(
        serde_json::json!({ "ok": true, "name": req.name, "value": v }),
    ))
}

async fn rt_unforce(
    State(state): State<AppState>,
    Json(req): Json<VarNameReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state
        .handle
        .unforce_variable(&req.name)
        .await
        .map_err(write_err)?;
    Ok(Json(serde_json::json!({ "ok": true, "name": req.name })))
}

async fn stop_handler(State(state): State<AppState>) -> impl IntoResponse {
    state.shutdown.notify_waiters();
    Json(serde_json::json!({"ok": true}))
}
