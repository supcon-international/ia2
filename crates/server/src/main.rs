mod edges;
mod error;
mod events;
mod routes;
mod state;

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use iomap_modbus::{run_demo_slave, DemoSlave};
use project::{load_last_opened, migrate_legacy_dirs, ProjectStore};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

/// Address the in-process demo Modbus TCP slave binds to. Override with
/// `DEMO_MODBUS_ADDR=host:port`; set to an empty string to disable the
/// slave entirely (useful when port 5502 is taken by something else).
const DEFAULT_DEMO_MODBUS_ADDR: &str = "127.0.0.1:5502";

/// Default bind for the HTTP server. `127.0.0.1:0` means "pick any free
/// port" — which is what the desktop shell uses so we never collide with
/// whatever else the user has running. The legacy `:3001` default lives
/// in dev scripts that the Vite proxy points at.
const DEFAULT_BIND: &str = "127.0.0.1:3001";

/// CLI flags for the long-lived HTTP backend. Deliberately tiny — every
/// flag here either makes the desktop shell possible (`--bind 0`,
/// `--print-url`, `--static-dir`) or surfaces a knob that already
/// existed as an env var (`--demo-modbus-addr` mirrors
/// `DEMO_MODBUS_ADDR`). New flags need a rationale; we are not building
/// a CLI here, just an embeddable backend.
#[derive(Parser, Debug)]
#[command(
    name = "ia2-server",
    about = "HTTP backend for the IA2 IDE (axum + ironplc bridge + iomap)."
)]
struct Cli {
    /// Address to bind. Use `127.0.0.1:0` to let the OS pick a free port;
    /// combine with `--print-url` so a parent process (e.g. the macOS
    /// shell) can read the actual port from stdout. Default: 127.0.0.1:3001.
    #[arg(long, value_name = "ADDR", default_value = DEFAULT_BIND)]
    bind: String,

    /// Print the actual bound URL (e.g. `http://127.0.0.1:54321`) to
    /// stdout once the listener is ready, on its own line. The native
    /// shell parses this to know where to point its WebView. No-op when
    /// you're running the server interactively.
    #[arg(long)]
    print_url: bool,

    /// Directory containing a pre-built `apps/web/dist` to serve at `/`.
    /// When omitted, only the JSON API is exposed (which is the current
    /// `vite dev` behaviour — Vite serves the React app, proxies `/api`
    /// here). When set, the server becomes a single origin hosting both
    /// the UI and the API, which is what the desktop shell points its
    /// WebView at. A missing/invalid path is a hard error — better to
    /// fail loudly than to silently 404 every page request.
    #[arg(long, value_name = "DIR")]
    static_dir: Option<PathBuf>,

    /// Override the demo Modbus slave address. Equivalent to the
    /// `DEMO_MODBUS_ADDR` env var; flag wins when both are set. Pass
    /// an empty string to disable the slave.
    #[arg(long, value_name = "ADDR")]
    demo_modbus_addr: Option<String>,

    /// If set, periodically check whether the given PID is still
    /// alive and exit if not. The Mac/Windows desktop shell passes
    /// its own PID here so the server reaps itself if the shell is
    /// SIGKILLed, panics, or is otherwise reaped without a chance to
    /// run cleanup. Without this, the server would orphan and keep
    /// the port + project lock indefinitely. Set to 0 / unset to
    /// disable.
    #[arg(long, value_name = "PID")]
    parent_pid: Option<i32>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Spawn the parent-liveness watchdog at the *very top* of main,
    // before tracing/subscribers/runtime setup. We deliberately don't
    // tuck it next to its sibling concerns later in the file — see
    // commit history: when spawned after `axum::serve(...).await` had
    // initialized its state, the std::thread::spawn was silently
    // unreachable in macOS-launchd-launched processes (the spawn line
    // never executed; main's sync code path between bind and serve
    // was effectively skipped). Moving it ahead of *all* other
    // initialization made it reliable. The cost is one thread parked
    // in read(2); pay it.
    if cli.parent_pid.is_some() {
        spawn_parent_watchdog();
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("server=debug,tower_http=info,info")),
        )
        // The desktop shell parses stdout to discover the URL. Anything
        // tracing emits goes to stderr to keep stdout clean — that's the
        // default for `tracing_subscriber::fmt()` but pin it explicitly
        // so a future refactor doesn't accidentally break the shell.
        .with_writer(std::io::stderr)
        .init();

    // One-time legacy-path migration: rename `~/Documents/controlsoftware`
    // → `~/Documents/IA2` (and same for config dir) if the legacy
    // directories exist. No-op once migrated; cheap on every start.
    migrate_legacy_dirs();

    // Spin up the demo Modbus slave so users can wire a Modbus device
    // against it without external hardware. The slave is shared with
    // AppState so the frontend can peek register/coil values.
    let demo_slave = DemoSlave::new();
    let demo_addr = cli
        .demo_modbus_addr
        .clone()
        .or_else(|| std::env::var("DEMO_MODBUS_ADDR").ok())
        .unwrap_or_else(|| DEFAULT_DEMO_MODBUS_ADDR.into());
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
        tracing::info!("demo modbus slave disabled");
    }

    let mut app = Router::new()
        .route("/health", get(routes::health))
        .route("/api/health", get(routes::api_health))
        // Project lifecycle
        .route(
            "/api/projects",
            get(routes::list_projects).post(routes::create_project),
        )
        .route("/api/projects/open", post(routes::open_project))
        .route("/api/projects/close", post(routes::close_project))
        // Returns every project the server currently has open, plus
        // which one is the active fallback. The multi-window IDE
        // calls this on new-window to populate its project picker.
        .route("/api/projects/open-list", get(routes::list_open_projects))
        .route("/api/project", get(routes::project_tree))
        .route("/api/project/validate", post(routes::validate_project))
        .route("/api/project/variables", get(routes::project_variables))
        .route("/api/project/pous", get(routes::project_pous))
        // POUs — file-level CRUD. A `.st` file may declare multiple IEC
        // POUs (PROGRAM + FB + FUNCTION side by side); declarations are
        // parsed and surfaced in /api/project + /api/project/pous.
        .route("/api/pous", post(routes::create_pou))
        .route("/api/pous/folders", post(routes::create_pou_folder))
        .route(
            "/api/pous/folders/{*path}",
            axum::routing::delete(routes::delete_pou_folder),
        )
        .route(
            "/api/pous/{path}",
            get(routes::get_pou)
                .put(routes::save_pou)
                .delete(routes::delete_pou),
        )
        .route("/api/pous/{path}/variables", get(routes::pou_variables))
        // Devices
        .route("/api/devices", post(routes::create_device))
        .route("/api/devices/folders", post(routes::create_device_folder))
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
        .route("/api/edges/folders", post(routes::create_edge_folder))
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
        .route("/api/symbols", post(routes::symbols))
        .route("/api/run", post(routes::run))
        .route("/api/stop", post(routes::stop))
        .route("/api/events", get(routes::events))
        // Synchronous runtime queries — agent-friendly alternatives to SSE.
        .route("/api/runtime/snapshot", get(routes::runtime_snapshot))
        .route("/api/runtime/status", get(routes::runtime_status))
        // Debug control trio
        .route("/api/runtime/pause", post(routes::runtime_pause))
        .route("/api/runtime/resume", post(routes::runtime_resume))
        .route("/api/runtime/step", post(routes::runtime_step))
        .route("/api/runtime/forces", get(routes::list_runtime_forces))
        .route(
            "/api/runtime/forces/{name}",
            post(routes::force_runtime_variable).delete(routes::unforce_runtime_variable),
        )
        .route(
            "/api/runtime/variables/{name}",
            post(routes::write_runtime_variable),
        )
        // Agent activity heartbeat — transient one-off path; the
        // overlay flashes on then ages out after TRANSIENT_TTL.
        // See crates/server/src/events.rs::AgentActivity for the
        // takeover-overlay protocol.
        .route("/api/agent/heartbeat", post(routes::agent_heartbeat))
        // Explicit session enter / leave. The recommended path for
        // any multi-step agent workflow — the overlay stays on for
        // the full duration with the agent-supplied label rather
        // than flickering between commands.
        .route(
            "/api/agent/session/start",
            post(routes::start_agent_session),
        )
        .route("/api/agent/session/end", post(routes::end_agent_session))
        // LSP bridge — WebSocket-upgraded; the browser-side monaco-
        // languageclient connects here and talks LSP JSON-RPC to a
        // freshly-spawned ironplc LSP process.
        .route("/api/lsp", get(routes::lsp))
        // Internal: peek + poke the in-process demo Modbus slave.
        .route("/api/_demo/slave", get(routes::demo_slave))
        .route(
            "/api/_demo/slave/{kind}/{addr}",
            axum::routing::put(routes::poke_demo_slave),
        );

    // Optionally serve the built React app at `/`. ServeDir handles
    // SPA-style 404→index.html via `not_found_service`, so the
    // TanStack-router client routes ("/", "/settings", etc.) all
    // resolve to the same React bundle. API routes shadow `/api/*`
    // regardless of static-dir state because they were registered
    // first.
    if let Some(dir) = &cli.static_dir {
        if !dir.is_dir() {
            anyhow::bail!(
                "--static-dir {:?} is not a directory (did you `pnpm --filter @cs/web build`?)",
                dir
            );
        }
        let index = dir.join("index.html");
        if !index.is_file() {
            anyhow::bail!(
                "--static-dir {:?} has no index.html — expected a Vite build output",
                dir
            );
        }
        tracing::info!(static_dir = %dir.display(), "serving static UI");
        // SPA pattern in two layers:
        //   1. ServeDir resolves real files under the dist tree
        //      (`/assets/index-XYZ.js`, `/favicon.ico`, etc).
        //   2. When ServeDir can't find a file, we fall through to a
        //      handler that reads index.html and returns it with a
        //      200 status — so a hard refresh on a client route like
        //      `/settings` re-bootstraps the SPA cleanly instead of
        //      surfacing a 404 (which would also pollute crash
        //      reporting and analytics with bogus "not found" hits).
        //
        // Note: this means a typo in an API path (e.g. `/api/poos`
        // instead of `/api/pous`) returns the React shell with 200.
        // The network tab makes that mistake obvious — small cost for
        // a single-origin setup.
        let index_html = index.clone();
        let spa_fallback = axum::routing::any(move || {
            let path = index_html.clone();
            async move { spa_index(path).await }
        });
        let serve = ServeDir::new(dir).fallback(spa_fallback);
        app = app.fallback_service(serve);
    }

    // Clone the state for the agent-activity watchdog before
    // `.with_state` consumes it. The watchdog runs forever on a
    // tokio task and only ever reads agent.lock() — sharing the
    // same Arc<Mutex<...>> with the request handlers is correct.
    let state_for_watchdog = state.clone();

    let app = app
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let bind_addr: SocketAddr = cli
        .bind
        .parse()
        .map_err(|e| anyhow::anyhow!("--bind {:?} is not a SocketAddr: {e}", cli.bind))?;
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local = listener.local_addr()?;
    tracing::info!(addr = %local, "server listening");
    if cli.print_url {
        // Single line, no prefix, no trailing whitespace beyond \n.
        // The shell's BackendSupervisor reads exactly one line and
        // treats it as the base URL. Anything else (tracing logs,
        // panic backtraces) goes to stderr.
        println!("http://{}", local);
    }

    // (parent-liveness watchdog spawned earlier at the very top of
    // main — see comment there for why)

    // Agent-activity watchdog: every 500 ms, check whether the last
    // CLI heartbeat aged out past the TTL; if so, flip `active=false`
    // and emit an AgentActivity event so the IDE drops its takeover
    // overlay. Cheap (one mutex peek + maybe one broadcast send).
    tokio::spawn(agent_watchdog(state_for_watchdog));

    axum::serve(listener, app).await?;
    Ok(())
}

/// Trailing edge of the agent-activity flag. Runs forever; cheap; the
/// leading edge (active=true) is handled inline by
/// `AppState::record_agent_heartbeat` / `start_agent_session`.
///
/// Two distinct timeouts:
///   - `TRANSIENT_TTL` (3 s) ages out individual heartbeats from
///     one-off `cs` commands. After this much idle, the overlay
///     flashes off.
///   - `SESSION_TTL` (30 s) ages out an open session whose agent
///     stopped heartbeat-pinging (process crashed, network
///     dropped). Generous so a slow-running agent doesn't get
///     kicked mid-thought.
async fn agent_watchdog(state: AppState) {
    const TRANSIENT_TTL: std::time::Duration = std::time::Duration::from_secs(3);
    const SESSION_TTL: std::time::Duration = std::time::Duration::from_secs(30);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let event = {
            let mut s = state.agent.lock().expect("agent mutex");
            if !s.active {
                continue;
            }
            // Session crash recovery: end any session that hasn't
            // heartbeat-pinged in SESSION_TTL. This is the only
            // place the watchdog touches `s.session`.
            let session_expired = s
                .session
                .as_ref()
                .is_some_and(|sess| sess.last_heartbeat.elapsed() >= SESSION_TTL);
            if session_expired {
                tracing::warn!(
                    "agent session expired (no heartbeat for {SESSION_TTL:?}); auto-ending"
                );
                s.session = None;
            }
            // After potentially clearing the session, decide if the
            // public `active` flag should drop. A session being
            // open pins active=true regardless of heartbeats.
            if s.session.is_some() {
                continue;
            }
            // No session — fall back to transient-heartbeat aging.
            // If `active` is set with no heartbeat to age against,
            // that's a state-shape bug; clear active so we exit the
            // hot path next tick instead of looping.
            match s.last_heartbeat.map(|h| h.elapsed()) {
                Some(e) if e >= TRANSIENT_TTL => {
                    s.active = false;
                    Some(events::AppEvent::AgentActivity(events::AgentActivity {
                        active: false,
                        command: s.command.clone(),
                        session: s.session_hint.clone(),
                        session_label: None,
                        since_ms: e.as_millis() as u64,
                    }))
                }
                None => {
                    s.active = false;
                    None
                }
                _ => None,
            }
        };
        if let Some(ev) = event {
            let _ = state.event_tx.send(ev);
        }
    }
}

/// Watch the parent shell for liveness. Strategy: block on a read
/// from stdin and exit when it returns 0 (EOF). The Mac/Windows
/// shell never writes anything to our stdin while running, so the
/// blocking read just sits there parked. The instant the shell
/// process dies — gracefully via SIGTERM, ungracefully via SIGKILL,
/// or via panic — the OS closes the write end of the pipe and our
/// read returns 0.
///
/// Why this instead of PPID polling?
///   - macOS GUI apps launched via `open` / launchd have a non-
///     obvious reparent timing window: `ps` shows ppid=1 instantly,
///     but `getppid()` inside the child can lag (observed: up to
///     several seconds, sometimes not at all in a single test run).
///     stdin EOF is detected by the kernel synchronously when the
///     pipe's write end is closed, with no zombie/launchd nuance.
///   - Works identically regardless of launch method (direct,
///     `open`, double-click in Finder, Spotlight, `launchctl`,
///     ssh forwarding).
///   - Future Linux `cs runtime` case gets the same behaviour for
///     free.
///
/// We use `libc::_exit` rather than `std::process::exit` so we skip
/// Rust runtime cleanup (atexit, destructors). The whole point is to
/// die fast and let the OS reclaim everything — there's nothing to
/// flush.
fn spawn_parent_watchdog() {
    std::thread::Builder::new()
        .name("parent-watchdog".into())
        .spawn(move || {
            tracing::info!("parent watchdog armed (stdin EOF detection)");
            let mut buf = [0u8; 64];
            loop {
                // SAFETY: read(2) on fd 0 is safe; we only care
                // about whether it returns 0 (EOF) or -1 (error).
                let rv = unsafe { libc::read(0, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                if rv <= 0 {
                    // EOF (0) or error (-1) — parent gone.
                    unsafe { libc::_exit(0) };
                }
                // The shell never writes to our stdin, so if we got
                // bytes it's a misuse — drop them and keep watching.
            }
        })
        .expect("spawn parent-watchdog thread");
}

/// Serve the SPA's index.html for any path that wasn't a real file
/// under `--static-dir`. Hard-coded 200 (cf. main(): we want client
/// router refresh-on-deep-link to look like a fresh visit, not a 404).
async fn spa_index(path: PathBuf) -> Response {
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            bytes,
        )
            .into_response(),
        Err(e) => {
            tracing::error!(path = %path.display(), %e, "failed to read SPA index.html");
            (StatusCode::INTERNAL_SERVER_ERROR, "index.html not readable").into_response()
        }
    }
}

fn try_open_last_project(state: &AppState) {
    // First, try restoring the full open-projects set from the
    // multi-project persistence file. If that file exists, it's the
    // authoritative source and we skip the legacy `last_opened` path
    // — multi-window users have multiple projects to restore.
    crate::routes::load_open_projects(state);
    if !state.projects.lock().expect("projects mutex").is_empty() {
        return;
    }
    // Legacy fallback (pre-multi-project IDE installations): the
    // single-project `last_opened` file still wins. Once opened, the
    // first `save_open_projects` triggered by any CRUD action
    // migrates the user forward.
    let Some(path) = load_last_opened() else {
        return;
    };
    match ProjectStore::open(path.clone()) {
        Ok(store) => {
            tracing::info!(path = %store.root().display(), "reopened last project");
            state
                .projects
                .lock()
                .expect("projects mutex")
                .insert_and_activate(store);
        }
        Err(e) => tracing::warn!(?path, %e, "failed to reopen last project"),
    }
}
