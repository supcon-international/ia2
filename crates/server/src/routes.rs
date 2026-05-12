//! HTTP route handlers.
//!
//! Grouped by concern (project lifecycle / POUs / devices / iomap / runtime
//! / health) but kept in one file because the layer is still small.

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use axum::{
    Json,
    extract::{
        Path as AxumPath, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::stream::Stream;
use futures_util::{SinkExt, StreamExt as FuturesStreamExt};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use ironplc_bridge::{CheckDiagnostic, DeviceSpec, VariableInfo};
use project::{
    Application, ApplicationKind, Device, Edge, IoMap, MigrationReport, ProjectListing,
    ProjectManifest, ProjectStore, ProjectTree, Protocol, Tasks, default_projects_dir,
    load_last_opened, save_last_opened,
};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt as TokioStreamExt;
use tokio_stream::wrappers::BroadcastStream;
use ts_rs::TS;

use crate::edges::{AttachInfo, DeployReport, EdgeProbe, attach_edge, deploy_to_edge, probe_edge};
use crate::error::ApiError;
use crate::events::AppEvent;
use crate::state::AppState;

// ============================================================
//  Response types (TS-exported)
// ============================================================

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct DemoSlaveSnapshot {
    pub coils: Vec<bool>,
    pub discrete_inputs: Vec<bool>,
    pub holding_registers: Vec<u16>,
    pub input_registers: Vec<u16>,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct HealthStatus {
    pub status: String,
    pub uptime_secs: u64,
    pub project_open: bool,
    pub program_running: bool,
    /// Where the bundled demo Modbus TCP slave is listening, or empty
    /// when disabled (DEMO_MODBUS_ADDR="").
    pub demo_modbus_addr: String,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct RunResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ProjectInfo {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct CreateProjectRequest {
    pub name: String,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct OpenProjectRequest {
    pub path: String,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct CreateApplicationRequest {
    pub name: String,
    pub kind: ApplicationKind,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct CreateDeviceRequest {
    pub name: String,
    pub protocol: Protocol,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct CreateFolderRequest {
    /// Forward-slash relative path under `applications/` or `devices/`,
    /// e.g. `"pid_loops"` or `"actuators/valves"`. Each segment must be
    /// non-empty, not start with '.', and contain no backslashes/colons.
    pub path: String,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct CreateEdgeRequest {
    pub name: String,
    /// SSH host or ~/.ssh/config alias. Anything ssh(1) would accept.
    pub host: String,
}

/// /api/run takes no body — the project's `tasks.toml` (+ all POU sources)
/// is now the single source of truth for "what runs". Kept as an opaque
/// type to preserve the route signature; future "dry-run" / "compile only"
/// flags can go here.
#[derive(Debug, Default, Deserialize, TS)]
#[ts(export)]
pub struct RunRequest {}

// ============================================================
//  Health
// ============================================================

pub async fn health(State(state): State<AppState>) -> Json<HealthStatus> {
    let elapsed = state.start_time.elapsed().as_secs();
    let project_open = state.project.lock().expect("project mutex").is_some();
    let program_running = state.program.lock().expect("program mutex").is_some();
    Json(HealthStatus {
        status: "ok".into(),
        uptime_secs: elapsed,
        project_open,
        program_running,
        demo_modbus_addr: state.demo_modbus_addr.clone(),
    })
}

// ============================================================
//  Project lifecycle
// ============================================================

pub async fn list_projects() -> Json<Vec<ProjectListing>> {
    Json(scan_projects())
}

pub async fn create_project(
    State(state): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<Json<ProjectInfo>, ApiError> {
    let root = default_projects_dir().join(&req.name);
    let store = ProjectStore::create(root, &req.name)?;
    let info = ProjectInfo {
        name: store.name().into(),
        path: store.root().display().to_string(),
    };
    save_last_opened(store.root());
    *state.project.lock().expect("project mutex") = Some(store);
    Ok(Json(info))
}

pub async fn open_project(
    State(state): State<AppState>,
    Json(req): Json<OpenProjectRequest>,
) -> Result<Json<ProjectInfo>, ApiError> {
    let path = PathBuf::from(req.path);
    let store = ProjectStore::open(path)?;
    let info = ProjectInfo {
        name: store.name().into(),
        path: store.root().display().to_string(),
    };
    save_last_opened(store.root());
    *state.project.lock().expect("project mutex") = Some(store);
    Ok(Json(info))
}

pub async fn close_project(State(state): State<AppState>) -> Json<RunResponse> {
    if let Some(handle) = state.program.lock().expect("program").take() {
        handle.stop();
    }
    *state.project.lock().expect("project") = None;
    Json(RunResponse { ok: true })
}

pub async fn project_tree(
    State(state): State<AppState>,
) -> Result<Json<ProjectTree>, ApiError> {
    with_project(&state, |store| store.tree().map_err(Into::into)).map(Json)
}

// ============================================================
//  Applications (POUs)
// ============================================================

pub async fn get_application(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<Application>, ApiError> {
    with_project(&state, |store| store.read_application(&name).map_err(Into::into)).map(Json)
}

pub async fn create_application(
    State(state): State<AppState>,
    Json(req): Json<CreateApplicationRequest>,
) -> Result<Json<Application>, ApiError> {
    with_project(&state, |store| {
        store
            .create_application(&req.name, req.kind)
            .map_err(Into::into)
    })
    .map(Json)
}

pub async fn save_application(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    body: String,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.write_application(&name, &body).map_err(Into::into)
    })?;
    Ok(Json(RunResponse { ok: true }))
}

pub async fn delete_application(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.delete_application(&name).map_err(Into::into)
    })?;
    Ok(Json(RunResponse { ok: true }))
}

pub async fn create_application_folder(
    State(state): State<AppState>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.create_application_folder(&req.path).map_err(Into::into)
    })?;
    Ok(Json(RunResponse { ok: true }))
}

/// Variables declared inside the named POU. Returns an empty list rather
/// than an error if the source can't be parsed — useful while the user is
/// mid-typing.
pub async fn application_variables(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<Vec<VariableInfo>>, ApiError> {
    let source = with_project(&state, |store| {
        store.read_application(&name).map_err(Into::into)
    })?
    .source;
    Ok(Json(ironplc_bridge::extract_variables(&source)))
}

// ============================================================
//  Devices
// ============================================================

pub async fn get_device(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<Device>, ApiError> {
    with_project(&state, |store| store.read_device(&name).map_err(Into::into)).map(Json)
}

pub async fn create_device(
    State(state): State<AppState>,
    Json(req): Json<CreateDeviceRequest>,
) -> Result<Json<Device>, ApiError> {
    with_project(&state, |store| {
        store
            .create_device(&req.name, req.protocol)
            .map_err(Into::into)
    })
    .map(Json)
}

pub async fn update_device(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(device): Json<Device>,
) -> Result<Json<RunResponse>, ApiError> {
    if device.name != name {
        return Err(ApiError::BadRequest(format!(
            "path name '{name}' does not match body name '{}'",
            device.name
        )));
    }
    with_project(&state, |store| store.write_device(&device).map_err(Into::into))?;
    Ok(Json(RunResponse { ok: true }))
}

pub async fn delete_device(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.delete_device(&name).map_err(Into::into)
    })?;
    Ok(Json(RunResponse { ok: true }))
}

pub async fn create_device_folder(
    State(state): State<AppState>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.create_device_folder(&req.path).map_err(Into::into)
    })?;
    Ok(Json(RunResponse { ok: true }))
}

// ============================================================
//  Edges (deploy targets)
// ============================================================

pub async fn get_edge(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<Edge>, ApiError> {
    with_project(&state, |store| store.read_edge(&name).map_err(Into::into)).map(Json)
}

pub async fn create_edge(
    State(state): State<AppState>,
    Json(req): Json<CreateEdgeRequest>,
) -> Result<Json<Edge>, ApiError> {
    with_project(&state, |store| {
        store.create_edge(&req.name, &req.host).map_err(Into::into)
    })
    .map(Json)
}

pub async fn update_edge(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(edge): Json<Edge>,
) -> Result<Json<RunResponse>, ApiError> {
    if edge.name != name {
        return Err(ApiError::BadRequest(format!(
            "path name '{name}' does not match body name '{}'",
            edge.name
        )));
    }
    with_project(&state, |store| store.write_edge(&edge).map_err(Into::into))?;
    Ok(Json(RunResponse { ok: true }))
}

pub async fn delete_edge(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| store.delete_edge(&name).map_err(Into::into))?;
    // Drop any active attachment for this edge so we don't leak ssh procs.
    state.attachments.detach(&name);
    Ok(Json(RunResponse { ok: true }))
}

pub async fn create_edge_folder(
    State(state): State<AppState>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.create_edge_folder(&req.path).map_err(Into::into)
    })?;
    Ok(Json(RunResponse { ok: true }))
}

pub async fn probe_edge_route(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<EdgeProbe>, ApiError> {
    let edge = with_project(&state, |store| store.read_edge(&name).map_err(Into::into))?;
    Ok(Json(probe_edge(&edge).await))
}

pub async fn deploy_edge_route(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<DeployReport>, ApiError> {
    let (edge, project_dir) = {
        let guard = state.project.lock().expect("project mutex");
        let store = guard.as_ref().ok_or(ApiError::NoProject)?;
        let edge = store
            .read_edge(&name)
            .map_err(|e| ApiError::from(crate::error::project_err(e)))?;
        (edge, store.root().to_path_buf())
    };
    let runtime_binary = find_runtime_binary();
    deploy_to_edge(&edge, &project_dir, runtime_binary.as_deref())
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn attach_edge_route(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<AttachInfo>, ApiError> {
    let edge = with_project(&state, |store| store.read_edge(&name).map_err(Into::into))?;
    attach_edge(&edge, &state.attachments)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn detach_edge_route(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Json<RunResponse> {
    state.attachments.detach(&name);
    Json(RunResponse { ok: true })
}

/// Convenience: the IDE keeps showing the local /api/events stream by
/// default; switching to an attached edge means changing the SSE source
/// URL on the client. Rather than build a streaming proxy, we tell the
/// client where to point — `attach` already returns the local port.
///
/// This endpoint lets the UI ask "is anything attached right now, and at
/// what port?" on page load (so a detach across reload is visible).
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct AttachmentStatus {
    pub attached: bool,
    pub local_port: Option<u16>,
}

pub async fn attachment_status(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Json<AttachmentStatus> {
    let port = state.attachments.current_port(&name);
    Json(AttachmentStatus {
        attached: port.is_some(),
        local_port: port,
    })
}

/// Look for a freshly-built runtime binary on the dev machine. Heuristic:
/// release first, then debug, then env var override. Returns None if no
/// binary is found — deploy falls back to "reuse current/runtime".
fn find_runtime_binary() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("CONTROLSOFTWARE_RUNTIME_BIN") {
        let p = std::path::PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?.to_path_buf();
    // Sibling of `server` binary in the same target dir.
    for candidate in [
        parent.join("controlsoftware-runtime"),
        parent.parent()?.join("release").join("controlsoftware-runtime"),
    ] {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

// ============================================================
//  IO Mapping
// ============================================================

pub async fn get_iomap(State(state): State<AppState>) -> Result<Json<IoMap>, ApiError> {
    with_project(&state, |store| store.read_iomap().map_err(Into::into)).map(Json)
}

pub async fn put_iomap(
    State(state): State<AppState>,
    Json(iomap): Json<IoMap>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| store.write_iomap(&iomap).map_err(Into::into))?;
    Ok(Json(RunResponse { ok: true }))
}

// ============================================================
//  Tasks (project-level scheduling)
// ============================================================

pub async fn get_tasks(State(state): State<AppState>) -> Result<Json<Tasks>, ApiError> {
    with_project(&state, |store| {
        Ok(store.read_tasks()?.unwrap_or_default())
    })
    .map(Json)
}

pub async fn put_tasks(
    State(state): State<AppState>,
    Json(tasks): Json<Tasks>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| store.write_tasks(&tasks).map_err(Into::into))?;
    Ok(Json(RunResponse { ok: true }))
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct MigrationResponse {
    pub migrated: bool,
    pub tasks_count: usize,
    pub programs_count: usize,
    pub pous_modified: Vec<String>,
}

/// Promote a legacy project (inline CONFIGURATION blocks in each POU) to
/// the new project-level `tasks.toml`. Idempotent — running on an
/// already-migrated project is a no-op.
pub async fn migrate_tasks(
    State(state): State<AppState>,
) -> Result<Json<MigrationResponse>, ApiError> {
    let report = with_project(&state, |store| store.migrate_tasks().map_err(Into::into))?;
    let resp = match report {
        MigrationReport::Skipped => MigrationResponse {
            migrated: false,
            tasks_count: 0,
            programs_count: 0,
            pous_modified: vec![],
        },
        MigrationReport::Migrated {
            tasks_count,
            programs_count,
            pous_modified,
        } => MigrationResponse {
            migrated: true,
            tasks_count,
            programs_count,
            pous_modified,
        },
    };
    Ok(Json(resp))
}

// ============================================================
//  Compile / run / stream
// ============================================================

pub async fn check(body: String) -> Json<Vec<CheckDiagnostic>> {
    Json(ironplc_bridge::check(&body))
}

pub async fn run(
    State(state): State<AppState>,
    _body: Option<Json<RunRequest>>,
) -> Result<Json<RunResponse>, ApiError> {
    // The whole project compiles+runs as one unit. POUs + tasks.toml +
    // devices + iomap all read from disk under one mutex hold.
    let (container, device_specs, mappings) = {
        let store_guard = state.project.lock().expect("project mutex");
        let store = store_guard.as_ref().ok_or(ApiError::NoProject)?;
        let container = ironplc_bridge::compile_project(store)?;
        let devices = store.list_devices()?;
        let iomap = store.read_iomap()?;
        let specs = devices
            .into_iter()
            .map(|d| DeviceSpec {
                name: d.name,
                config: d.config,
            })
            .collect::<Vec<_>>();
        (container, specs, iomap.mappings)
    };

    {
        let mut guard = state.program.lock().expect("program mutex");
        if let Some(old) = guard.take() {
            old.stop();
        }
    }

    let handle = ironplc_bridge::spawn(container, device_specs, mappings);
    let mut rx = handle.subscribe();
    let event_tx = state.event_tx.clone();

    tokio::spawn(async move {
        while let Ok(snap) = rx.recv().await {
            let _ = event_tx.send(AppEvent::Snapshot(snap));
        }
    });

    state
        .program
        .lock()
        .expect("program mutex")
        .replace(handle);
    let _ = state.event_tx.send(AppEvent::Started);

    Ok(Json(RunResponse { ok: true }))
}

pub async fn stop(State(state): State<AppState>) -> Json<RunResponse> {
    if let Some(handle) = state
        .program
        .lock()
        .expect("program mutex")
        .take()
    {
        handle.stop();
    }
    let _ = state.event_tx.send(AppEvent::Stopped);
    Json(RunResponse { ok: true })
}

/// First 32 entries of each address space in the in-process demo slave.
/// Useful for verifying that the scan loop wrote to / read from the bus.
pub async fn demo_slave(State(state): State<AppState>) -> Json<DemoSlaveSnapshot> {
    const N: usize = 32;
    let coils = state.demo_slave.coils().lock().unwrap()[..N].to_vec();
    let discrete_inputs = state.demo_slave.discrete_inputs().lock().unwrap()[..N].to_vec();
    let holding_registers = state.demo_slave.holding_registers().lock().unwrap()[..N].to_vec();
    let input_registers = state.demo_slave.input_registers().lock().unwrap()[..N].to_vec();
    Json(DemoSlaveSnapshot {
        coils,
        discrete_inputs,
        holding_registers,
        input_registers,
    })
}

pub async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_tx.subscribe();
    let stream = TokioStreamExt::filter_map(BroadcastStream::new(rx), |res| match res {
        Ok(event) => Event::default().json_data(&event).ok().map(Ok),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

// ============================================================
//  LSP WebSocket bridge
// ============================================================

/// Upgrade to WebSocket and bridge to a freshly-spawned ironplc LSP
/// process. WS frames are LSP JSON-RPC bodies (no Content-Length header);
/// the proxy adds/strips headers when talking to stdio.
pub async fn lsp(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_lsp_ws)
}

fn lsp_launcher_path() -> PathBuf {
    if let Ok(p) = std::env::var("LSP_LAUNCHER") {
        return PathBuf::from(p);
    }
    let mut path = std::env::current_exe().expect("current_exe");
    path.pop();
    path.push("lsp-launcher");
    path
}

async fn handle_lsp_ws(socket: WebSocket) {
    let cmd = lsp_launcher_path();
    let mut child = match Command::new(&cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(path = %cmd.display(), %e, "failed to spawn lsp-launcher");
            return;
        }
    };
    tracing::info!(path = %cmd.display(), "lsp-launcher spawned for ws client");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let (mut ws_tx, mut ws_rx) = FuturesStreamExt::split(socket);

    // WS → child stdin: add LSP Content-Length framing.
    let to_child = async move {
        while let Some(msg) = FuturesStreamExt::next(&mut ws_rx).await {
            let Ok(msg) = msg else { break };
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Close(_) => break,
                _ => continue,
            };
            let body = text.as_bytes();
            let header = format!("Content-Length: {}\r\n\r\n", body.len());
            if stdin.write_all(header.as_bytes()).await.is_err() {
                break;
            }
            if stdin.write_all(body).await.is_err() {
                break;
            }
            if stdin.flush().await.is_err() {
                break;
            }
        }
    };

    // child stdout → WS: strip LSP Content-Length framing.
    let from_child = async move {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut content_length: Option<usize> = None;
            // Read headers until empty line.
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) => return,
                    Ok(_) => {}
                    Err(_) => return,
                }
                let trimmed = line.trim_end_matches(['\r', '\n']);
                if trimmed.is_empty() {
                    break;
                }
                if let Some(v) = trimmed.strip_prefix("Content-Length:") {
                    content_length = v.trim().parse().ok();
                }
            }
            let Some(len) = content_length else {
                continue;
            };
            let mut body = vec![0u8; len];
            if reader.read_exact(&mut body).await.is_err() {
                return;
            }
            let Ok(text) = String::from_utf8(body) else {
                continue;
            };
            if ws_tx.send(Message::Text(text.into())).await.is_err() {
                return;
            }
        }
    };

    tokio::select! {
        _ = to_child => {}
        _ = from_child => {}
    }
    let _ = child.kill().await;
}

// ============================================================
//  Helpers
// ============================================================

fn with_project<T>(
    state: &AppState,
    f: impl FnOnce(&ProjectStore) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let guard = state.project.lock().expect("project mutex");
    let store = guard.as_ref().ok_or(ApiError::NoProject)?;
    f(store)
}

/// Walk the default projects dir and surface anything that looks like a
/// project. Also includes the last-opened path if it lives elsewhere.
fn scan_projects() -> Vec<ProjectListing> {
    let last = load_last_opened();
    let mut out = Vec::new();
    let default_dir = default_projects_dir();
    if default_dir.exists() {
        if let Ok(entries) = fs::read_dir(&default_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                if let Some(listing) = listing_for(&path, last.as_deref()) {
                    out.push(listing);
                }
            }
        }
    }
    // Include the last-opened project even if it's outside the default dir.
    if let Some(ref last_path) = last {
        let already_listed = out.iter().any(|p| Path::new(&p.path) == last_path);
        if !already_listed {
            if let Some(listing) = listing_for(last_path, Some(last_path)) {
                out.push(listing);
            }
        }
    }
    out.sort_by(|a, b| {
        b.is_last_opened
            .cmp(&a.is_last_opened)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

fn listing_for(path: &Path, last_opened: Option<&Path>) -> Option<ProjectListing> {
    let manifest_path = path.join("project.toml");
    if !manifest_path.exists() {
        return None;
    }
    let text = fs::read_to_string(&manifest_path).ok()?;
    let manifest: ProjectManifest = toml::from_str(&text).ok()?;
    Some(ProjectListing {
        name: manifest.name,
        path: path.display().to_string(),
        is_last_opened: last_opened == Some(path),
    })
}
