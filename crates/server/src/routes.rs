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
        Path as AxumPath, Query, State,
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
use ironplc_bridge::{
    CheckDiagnostic, DeviceSpec, ProgramHandle, RuntimeMode, RuntimeWriteError, VarSnapshot,
    VariableInfo,
};
use project::{
    Device, Edge, IoMap, MigrationReport, Pou, PouFile, PouLanguage, PouType, ProgramInstance,
    ProjectListing, ProjectManifest, ProjectStore, ProjectTree, Protocol, Task, Tasks,
    default_projects_dir, load_last_opened, save_last_opened,
};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt as TokioStreamExt;
use tokio_stream::wrappers::BroadcastStream;
use ts_rs::TS;

use crate::edges::{AttachInfo, DeployReport, EdgeProbe, attach_edge, deploy_to_edge, probe_edge};
use crate::error::ApiError;
use crate::events::{AppEvent, MutationDetail, topic};
use serde::Deserialize as SerdeDeserialize;

#[derive(SerdeDeserialize, Debug, Default)]
pub struct AgentHeartbeatRequest {
    /// Short label for *what* the agent is currently doing — usually
    /// the `cs` subcommand path (e.g. "pou create"). Optional so a
    /// minimal "I'm alive" beacon works too.
    #[serde(default)]
    pub command: Option<String>,
    /// Stable identifier for this CLI run. The CLI generates a UUID
    /// once at process start and reuses it for every subsequent
    /// command, letting the frontend tell apart a single agent doing
    /// many things from multiple concurrent agents.
    #[serde(default)]
    pub session: Option<String>,
}

/// Heartbeat from an external agent (typically `cs` CLI). Updates the
/// "agent active" state used by the IDE's takeover overlay. Returns
/// instantly; the actual SSE event broadcast happens inside
/// `record_agent_heartbeat` on the leading edge, and inside the
/// `agent_watchdog` task on the trailing edge.
///
/// Returns the canonical `{ok: true}` to keep the wire-shape
/// consistent with other agent-friendly endpoints.
pub async fn agent_heartbeat(
    State(state): State<AppState>,
    Json(req): Json<AgentHeartbeatRequest>,
) -> Json<RunResponse> {
    state.record_agent_heartbeat(req.command, req.session);
    Json(RunResponse { ok: true })
}
use crate::state::{AppState, RunningInfo};

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
pub struct CreatePouRequest {
    /// Project-relative slash-path under `pous/`, without `.st`. The
    /// leaf is also used as the IEC POU identifier in the seeded source.
    pub path: String,
    #[serde(rename = "type")]
    pub type_: PouType,
    pub language: PouLanguage,
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

/// Body for /api/run. Two modes:
///
/// - `{}` (empty) — uses the project's `tasks.toml` as the schedule.
///   Runs every PROGRAM instance declared there. Used by the Tasks
///   pane's Run button and any agent that wants the production schedule.
///
/// - `{ "program": "<iec_name>" }` — ad-hoc single-PROGRAM run. The
///   server synthesizes a minimal schedule (one default task + one
///   instance of the named PROGRAM) without touching tasks.toml.
///   Used by the ProgramPane's Run button so "click Run while looking
///   at cascade_pid" runs cascade_pid, period — matching engineer
///   intuition even when tasks.toml schedules something else.
#[derive(Debug, Default, Deserialize, TS)]
#[ts(export)]
pub struct RunRequest {
    /// IEC POU identifier of the PROGRAM to run in ad-hoc mode. None =
    /// fall back to tasks.toml.
    #[serde(default)]
    pub program: Option<String>,
    /// POU file path containing `program`. When set together with
    /// `program`, the compile input is limited to this file alone —
    /// other POU files are not concatenated, so ironplc's debug section
    /// (and therefore the Monitor pane) only sees the running PROGRAM's
    /// variables, not those of unrelated PROGRAMs in other files.
    ///
    /// Ignored when `program` is None.
    #[serde(default)]
    pub file_path: Option<String>,
}

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

/// /api/health alias — same payload as /health. Exists so agents that
/// scope to /api/* don't have to special-case liveness.
pub async fn api_health(State(state): State<AppState>) -> Json<HealthStatus> {
    health(State(state)).await
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
    state.emit_mutation(
        topic::PROJECT_META,
        MutationDetail::ProjectCreated {
            name: info.name.clone(),
            path: info.path.clone(),
        },
    );
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
    state.emit_mutation(
        topic::PROJECT_META,
        MutationDetail::ProjectOpened {
            name: info.name.clone(),
            path: info.path.clone(),
        },
    );
    Ok(Json(info))
}

pub async fn close_project(State(state): State<AppState>) -> Json<RunResponse> {
    if let Some(handle) = state.program.lock().expect("program").take() {
        handle.stop();
    }
    *state.project.lock().expect("project") = None;
    // Wipe runtime caches — the data belonged to the project that's
    // being closed, and stale-looking values across projects would
    // confuse anyone hitting /api/runtime/snapshot.
    *state.last_snapshot.lock().expect("last_snapshot") = None;
    *state.last_error.lock().expect("last_error") = None;
    state.emit_mutation(topic::PROJECT_META, MutationDetail::ProjectClosed);
    Json(RunResponse { ok: true })
}

pub async fn project_tree(
    State(state): State<AppState>,
) -> Result<Json<ProjectTree>, ApiError> {
    with_project(&state, |store| {
        // The skeleton has each POU file's raw source. We parse each here
        // to surface declarations to the frontend (the store stays parser-
        // free; the bridge owns the parser).
        let skel = store.tree_skeleton()?;
        let pous: Vec<PouFile> = skel
            .pous
            .into_iter()
            .map(|f| PouFile {
                path: f.path,
                declarations: ironplc_bridge::extract_pou_declarations(&f.source, f.language),
            })
            .collect();
        Ok(ProjectTree {
            name: skel.name,
            path: skel.path,
            pous,
            pou_folders: skel.pou_folders,
            devices: skel.devices,
            device_folders: skel.device_folders,
            edges: skel.edges,
            edge_folders: skel.edge_folders,
            iomap: skel.iomap,
            tasks: skel.tasks,
        })
    })
    .map(Json)
}

// ============================================================
//  POUs (files holding 1+ IEC POU declarations)
// ============================================================
//
// Identifier convention: `path` = slash-separated location under
// `pous/`, no `.st` extension. Same shape as the old
// `/api/applications/{name}` route — only the noun changes.

/// Read a POU file: raw source + parsed declarations (PROGRAM/FB/FN found
/// inside). The IDE editor uses the source verbatim; agents and the Tasks
/// pane use the declaration list to drive scheduling.
pub async fn get_pou(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<Pou>, ApiError> {
    with_project(&state, |store| {
        let language = store.pou_file_language(&path)?;
        let source = store.read_pou_source(&path)?;
        Ok(Pou {
            path: path.clone(),
            declarations: ironplc_bridge::extract_pou_declarations(&source, language),
            source,
        })
    })
    .map(Json)
}

pub async fn create_pou(
    State(state): State<AppState>,
    Json(req): Json<CreatePouRequest>,
) -> Result<Json<Pou>, ApiError> {
    let created_path = req.path.clone();
    let pou = with_project(&state, |store| {
        let language = req.language;
        let source = store.create_pou_file(&req.path, req.type_, language)?;
        Ok(Pou {
            path: req.path,
            declarations: ironplc_bridge::extract_pou_declarations(&source, language),
            source,
        })
    })?;
    state.emit_mutation(
        topic::PROJECT,
        MutationDetail::PouCreated { path: created_path },
    );
    Ok(Json(pou))
}

pub async fn save_pou(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
    body: String,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.write_pou_source(&path, &body).map_err(Into::into)
    })?;
    // Fire two topics: the per-POU one (specific editor refetches its
    // own source) AND the project-wide one (declarations may have
    // changed, so the tree should re-derive symbols).
    state.emit_mutation(
        topic::pou(&path),
        MutationDetail::PouUpdated { path: path.clone() },
    );
    state.emit_mutation(
        topic::PROJECT,
        MutationDetail::PouUpdated { path },
    );
    Ok(Json(RunResponse { ok: true }))
}

pub async fn delete_pou(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| store.delete_pou_file(&path).map_err(Into::into))?;
    state.emit_mutation(
        topic::PROJECT,
        MutationDetail::PouDeleted { path },
    );
    Ok(Json(RunResponse { ok: true }))
}

pub async fn create_pou_folder(
    State(state): State<AppState>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.create_pou_folder(&req.path).map_err(Into::into)
    })?;
    state.emit_mutation(
        topic::PROJECT,
        MutationDetail::PouFolderCreated { path: req.path },
    );
    Ok(Json(RunResponse { ok: true }))
}

pub async fn delete_pou_folder(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.delete_pou_folder(&path).map_err(Into::into)
    })?;
    state.emit_mutation(
        topic::PROJECT,
        MutationDetail::PouFolderDeleted { path },
    );
    Ok(Json(RunResponse { ok: true }))
}

/// Variables declared inside any POU in this file. Empty list if parse
/// fails — handy for mid-typing editor calls.
pub async fn pou_variables(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<Vec<VariableInfo>>, ApiError> {
    let source = with_project(&state, |store| {
        store.read_pou_source(&path).map_err(Into::into)
    })?;
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
    let device = with_project(&state, |store| {
        store
            .create_device(&req.name, req.protocol)
            .map_err(Into::into)
    })?;
    state.emit_mutation(
        topic::DEVICES,
        MutationDetail::DeviceUpserted {
            name: device.name.clone(),
        },
    );
    Ok(Json(device))
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
    state.emit_mutation(
        topic::DEVICES,
        MutationDetail::DeviceUpserted { name: name.clone() },
    );
    state.emit_mutation(
        topic::device(&name),
        MutationDetail::DeviceUpserted { name },
    );
    Ok(Json(RunResponse { ok: true }))
}

pub async fn delete_device(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.delete_device(&name).map_err(Into::into)
    })?;
    state.emit_mutation(
        topic::DEVICES,
        MutationDetail::DeviceDeleted { name },
    );
    Ok(Json(RunResponse { ok: true }))
}

pub async fn create_device_folder(
    State(state): State<AppState>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.create_device_folder(&req.path).map_err(Into::into)
    })?;
    state.emit_mutation(
        topic::DEVICES,
        MutationDetail::DeviceFolderCreated { path: req.path },
    );
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
    let edge = with_project(&state, |store| {
        store.create_edge(&req.name, &req.host).map_err(Into::into)
    })?;
    state.emit_mutation(
        topic::EDGES,
        MutationDetail::EdgeUpserted {
            name: edge.name.clone(),
        },
    );
    Ok(Json(edge))
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
    state.emit_mutation(
        topic::EDGES,
        MutationDetail::EdgeUpserted { name: name.clone() },
    );
    state.emit_mutation(
        topic::edge(&name),
        MutationDetail::EdgeUpserted { name },
    );
    Ok(Json(RunResponse { ok: true }))
}

pub async fn delete_edge(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| store.delete_edge(&name).map_err(Into::into))?;
    // Drop any active attachment for this edge so we don't leak ssh procs.
    state.attachments.detach(&name);
    state.emit_mutation(
        topic::EDGES,
        MutationDetail::EdgeDeleted { name },
    );
    Ok(Json(RunResponse { ok: true }))
}

pub async fn create_edge_folder(
    State(state): State<AppState>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.create_edge_folder(&req.path).map_err(Into::into)
    })?;
    state.emit_mutation(
        topic::EDGES,
        MutationDetail::EdgeFolderCreated { path: req.path },
    );
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
    let info = attach_edge(&edge, &state.attachments)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    state.emit_mutation(
        topic::edge(&name),
        MutationDetail::EdgeAttached {
            name,
            local_port: info.local_port,
        },
    );
    Ok(Json(info))
}

pub async fn detach_edge_route(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Json<RunResponse> {
    state.attachments.detach(&name);
    state.emit_mutation(
        topic::edge(&name),
        MutationDetail::EdgeDetached { name },
    );
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
    if let Ok(p) = std::env::var("IA2_RUNTIME_BIN") {
        let p = std::path::PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?.to_path_buf();
    // Sibling of `server` binary in the same target dir.
    for candidate in [
        parent.join("ia2-runtime"),
        parent.parent()?.join("release").join("ia2-runtime"),
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
    state.emit_mutation(topic::IOMAP, MutationDetail::IoMapChanged);
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
    state.emit_mutation(topic::TASKS, MutationDetail::TasksChanged);
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
    if resp.migrated {
        // Migration touches both task config and POU sources, so
        // invalidate the broad project key too. The targeted topics
        // keep individual panes responsive.
        state.emit_mutation(topic::TASKS, MutationDetail::TasksMigrated);
        state.emit_mutation(topic::PROJECT, MutationDetail::TasksMigrated);
    }
    Ok(Json(resp))
}

// ============================================================
//  Compile / run / stream
// ============================================================

/// Query parameters for `POST /api/check`. `language` defaults to ST
/// for back-compat with callers that don't know about LD yet (they POST
/// a plain text/plain body and get ST-mode behaviour).
#[derive(Debug, Default, Deserialize)]
pub struct CheckQuery {
    #[serde(default)]
    pub language: Option<String>,
}

/// Symbol extraction for the editor's hover / completion providers
/// and the `cs symbols` CLI subcommand. Takes the raw source on the
/// wire (so unsaved edits work) + a `language` query param identical
/// to `/api/check`.
///
/// Returns `[]` on parse failure — the front-end falls back to its
/// hard-coded keyword / FB-type lists.
pub async fn symbols(
    Query(q): Query<CheckQuery>,
    body: String,
) -> Json<Vec<ironplc_bridge::VariableInfo>> {
    let language = match q.language.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("ld") => PouLanguage::Ld,
        Some("fbd") => PouLanguage::Fbd,
        Some("sfc") => PouLanguage::Sfc,
        _ => PouLanguage::St,
    };
    Json(ironplc_bridge::extract_symbols(&body, language))
}

pub async fn check(
    Query(q): Query<CheckQuery>,
    body: String,
) -> Json<Vec<CheckDiagnostic>> {
    // The frontend posts `?language=ld` when the editor source on the
    // wire is `.ld.json`. Anything else (or absent) is treated as ST,
    // which is the historical behaviour. Keeping `check` and
    // `check_pou_source` as one HTTP route means the editor doesn't
    // need a second LSP-style channel.
    let language = match q.language.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("ld") => PouLanguage::Ld,
        Some("fbd") => PouLanguage::Fbd,
        Some("sfc") => PouLanguage::Sfc,
        _ => PouLanguage::St,
    };
    Json(ironplc_bridge::check_pou_source(&body, language))
}

/// Compile the whole project (every POU + tasks.toml-synthesized
/// CONFIGURATION) without spawning. Returns diagnostics from the parser
/// + analyzer + codegen pipeline. Empty list = clean; safe to Run.
///
/// Agent use-case: validate before Deploy. Cheaper than POST /api/run
/// because no bridge thread or devices are touched.
pub async fn validate_project(
    State(state): State<AppState>,
) -> Result<Json<Vec<CheckDiagnostic>>, ApiError> {
    with_project(&state, |store| {
        // compile_project returns Ok(Container) when clean, Err on any
        // problem. Convert the error into a single CheckDiagnostic for
        // the agent — full per-line diagnostics live in /api/check for
        // POU-level editing. compile_project's failure surface is one of:
        //  - missing tasks.toml programs
        //  - parser / analyzer / codegen diagnostics (synthetic source)
        //  - file read failures
        match ironplc_bridge::compile_project(store) {
            Ok(_) => Ok(vec![]),
            Err(e) => Ok(vec![CheckDiagnostic {
                severity: "error".into(),
                code: "project-validate".into(),
                message: e.to_string(),
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 1,
                context: vec![],
                related: vec![],
                explanation: None,
                ld_location: None,
                fbd_location: None,
                sfc_location: None,
            }]),
        }
    })
    .map(Json)
}

// ============================================================
//  Cross-POU declaration index (real schedulable POU names)
// ============================================================

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct PouInProject {
    /// File the declaration lives in — project-relative POU path
    /// (e.g. `pid_loops/temperature_pid`).
    pub file_path: String,
    /// IEC POU identifier — what `PROGRAM <inst> WITH <task> : <name>`
    /// references in a CONFIGURATION block.
    pub name: String,
    #[serde(rename = "type")]
    pub type_: PouType,
    pub language: PouLanguage,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ProjectPous {
    pub pous: Vec<PouInProject>,
}

/// Every IEC POU declared anywhere in the project, parser-driven so it
/// reflects what's actually schedulable. A single `.st` file may declare
/// multiple POUs (FB + PROGRAM + FUNCTION side by side); each appears
/// as its own entry here. Agents and the Tasks pane both use this to
/// populate the "PROGRAM to schedule" dropdown.
pub async fn project_pous(
    State(state): State<AppState>,
) -> Result<Json<ProjectPous>, ApiError> {
    with_project(&state, |store| {
        let paths = store.list_pou_paths()?;
        let mut out = Vec::new();
        for path in paths {
            let language = store.pou_file_language(&path)?;
            let source = store.read_pou_source(&path)?;
            for d in ironplc_bridge::extract_pou_declarations(&source, language) {
                out.push(PouInProject {
                    file_path: path.clone(),
                    name: d.name,
                    type_: d.type_,
                    language: d.language,
                });
            }
        }
        Ok(ProjectPous { pous: out })
    })
    .map(Json)
}

// ============================================================
//  Cross-POU variable index
// ============================================================

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ProjectVariable {
    /// POU file the variable was declared in (`pous/<file_path>.st`).
    pub file_path: String,
    pub name: String,
    pub type_name: String,
    pub direction: String,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ProjectVariables {
    pub variables: Vec<ProjectVariable>,
}

/// All variables declared anywhere in the project, qualified by their
/// owning POU. Used by debug agents to answer "which POU declares
/// `counter`?" / "list every BOOL variable" without round-tripping
/// per-POU.
pub async fn project_variables(
    State(state): State<AppState>,
) -> Result<Json<ProjectVariables>, ApiError> {
    with_project(&state, |store| {
        let paths = store.list_pou_paths()?;
        let mut out = Vec::new();
        for path in paths {
            let source = store.read_pou_source(&path)?;
            for v in ironplc_bridge::extract_variables(&source) {
                out.push(ProjectVariable {
                    file_path: path.clone(),
                    name: v.name,
                    type_name: v.type_name,
                    direction: v.direction,
                });
            }
        }
        Ok(ProjectVariables { variables: out })
    })
    .map(Json)
}

pub async fn run(
    State(state): State<AppState>,
    body: Option<Json<RunRequest>>,
) -> Result<Json<RunResponse>, ApiError> {
    let req = body.map(|Json(b)| b).unwrap_or_default();

    // Two modes (matched in the handler so the bridge stays simple):
    //  - `program: None`           → compile_project (reads tasks.toml)
    //  - `program: Some("foo")`    → compile_project_with_tasks (synthetic
    //                                 single-instance schedule; tasks.toml
    //                                 untouched on disk)
    let (container, device_specs, mappings) = {
        let store_guard = state.project.lock().expect("project mutex");
        let store = store_guard.as_ref().ok_or(ApiError::NoProject)?;
        let container = match (req.program.as_deref(), req.file_path.as_deref()) {
            (None, _) => ironplc_bridge::compile_project(store)?,
            (Some(name), Some(file_path)) => {
                // Ad-hoc isolated run: compile only the named file's
                // source + a single-PROGRAM CONFIGURATION. ironplc's
                // debug section then only knows about this file's
                // declarations, so Monitor + /api/runtime/snapshot show
                // exactly the variables the user is looking at.
                let language = store.pou_file_language(file_path)?;
                let source = store.read_pou_source(file_path)?;
                let tasks = single_program_tasks(name);
                ironplc_bridge::compile_isolated_source(&source, language, &tasks)?
            }
            (Some(name), None) => {
                // Ad-hoc but no file scope — fall back to whole-project
                // concatenation with a single-PROGRAM schedule. Other
                // files' variables WILL bleed into the debug section
                // (ironplc limitation); document this if a client hits
                // it.
                let tasks = single_program_tasks(name);
                ironplc_bridge::compile_project_with_tasks(store, &tasks)?
            }
        };
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
    let last_snapshot_cache = state.last_snapshot.clone();
    // Fresh run wipes the prior error; if this run errors, the SSE error
    // event will refill it.
    *state.last_error.lock().expect("last_error mutex") = None;

    tokio::spawn(async move {
        while let Ok(snap) = rx.recv().await {
            *last_snapshot_cache.lock().expect("last_snapshot mutex") =
                Some(snap.clone());
            let _ = event_tx.send(AppEvent::Snapshot(snap));
        }
    });

    state
        .program
        .lock()
        .expect("program mutex")
        .replace(handle);

    // Record what kind of run this is so /api/runtime/status can label
    // the Monitor pane on a fresh page load (which would otherwise have
    // no way to know — the SSE `Started` event already fired).
    let info = match (req.program.as_deref(), req.file_path.as_deref()) {
        (Some(name), Some(file_path)) => Some(RunningInfo::Isolated {
            program: name.to_string(),
            file_path: file_path.to_string(),
        }),
        (Some(name), None) => Some(RunningInfo::Scheduled {
            programs: vec![name.to_string()],
        }),
        (None, _) => {
            // Whole-project schedule — pull the PROGRAM names from
            // tasks.toml so the IDE can render them, not the instance
            // names (instances are bookkeeping; PROGRAM names are what
            // humans recognise from the POU tree).
            let programs = state
                .project
                .lock()
                .expect("project mutex")
                .as_ref()
                .and_then(|s| s.read_tasks().ok().flatten())
                .map(|t| t.programs.into_iter().map(|p| p.program).collect())
                .unwrap_or_default();
            Some(RunningInfo::Scheduled { programs })
        }
    };
    *state.running_info.lock().expect("running_info mutex") = info;

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
    *state.running_info.lock().expect("running_info mutex") = None;
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
//  Runtime — synchronous queries + variable writes
// ============================================================

/// Most recent VarSnapshot from the running bridge, or `null` when
/// nothing has been snapshotted in the current session (no run, or
/// project was just closed). Lets agents poll one-shot without
/// subscribing to /api/events SSE.
pub async fn runtime_snapshot(
    State(state): State<AppState>,
) -> Json<Option<VarSnapshot>> {
    Json(state.last_snapshot.lock().expect("last_snapshot").clone())
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct RuntimeStatus {
    pub running: bool,
    pub project: Option<String>,
    /// Program instances declared in tasks.toml (what's actually scheduled).
    pub program_instances: Vec<String>,
    pub devices: Vec<String>,
    /// Scan count from the most recent snapshot; 0 before the first one.
    pub scan_count: u64,
    /// Timestamp_us of the most recent snapshot, or 0.
    pub last_snapshot_us: u64,
    pub last_error: Option<String>,
    /// What kind of run is active (isolated vs scheduled) and which
    /// PROGRAM(s) it covers. Populated from `AppState.running_info`
    /// which the /api/run handler writes when it starts a program;
    /// cleared on /api/stop and on close-project. `None` here just
    /// means nothing is currently running.
    pub running_info: Option<RunningInfo>,
    /// Current scan-loop mode (`running` / `paused` / `step{remaining}`).
    /// `None` when nothing's running.
    pub mode: Option<RuntimeMode>,
    /// Currently-forced variables in `name=value` pairs, sorted by name.
    /// Empty when no force is active. The shape matches the in-memory
    /// HashMap snapshot from `ProgramHandle::forces()`.
    pub forces: Vec<ForceEntry>,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ForceEntry {
    pub name: String,
    pub value: i32,
}

/// One-shot overview of the runtime — designed for agents who want
/// "what's going on right now" without composing /health + /api/project
/// + the SSE stream.
pub async fn runtime_status(
    State(state): State<AppState>,
) -> Json<RuntimeStatus> {
    let project_open = state.project.lock().expect("project").is_some();
    let running = state.program.lock().expect("program").is_some();
    let (project_name, programs, devices) = {
        let guard = state.project.lock().expect("project");
        match guard.as_ref() {
            Some(store) => {
                let tasks = store.read_tasks().ok().flatten().unwrap_or_default();
                let programs = tasks
                    .programs
                    .iter()
                    .map(|p| p.instance.clone())
                    .collect();
                let devices = store
                    .list_devices()
                    .map(|ds| ds.iter().map(|d| d.name.clone()).collect())
                    .unwrap_or_default();
                (Some(store.name().to_string()), programs, devices)
            }
            None => (None, vec![], vec![]),
        }
    };
    let snap = state.last_snapshot.lock().expect("last_snapshot").clone();
    let last_error = state.last_error.lock().expect("last_error").clone();
    let running_info = state.running_info.lock().expect("running_info").clone();
    let _ = project_open; // suppress unused — kept for symmetry with runtime
    // Mode + forces come from the live ProgramHandle, when there is
    // one. Clone the handle out of the mutex briefly to avoid holding
    // the sync lock across the calls.
    let (mode, forces) = {
        let guard = state.program.lock().expect("program");
        match guard.as_ref() {
            Some(h) => {
                let forces = h
                    .forces()
                    .into_iter()
                    .map(|(name, value)| ForceEntry { name, value })
                    .collect();
                (Some(h.mode()), forces)
            }
            None => (None, vec![]),
        }
    };
    Json(RuntimeStatus {
        running,
        project: project_name,
        program_instances: programs,
        devices,
        scan_count: snap.as_ref().map(|s| s.scan_count).unwrap_or(0),
        last_snapshot_us: snap.as_ref().map(|s| s.timestamp_us).unwrap_or(0),
        last_error,
        running_info,
        mode,
        forces,
    })
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct WriteVariableRequest {
    /// Raw i32 value to write — the VM's variable-write primitive is
    /// `write_variable(VarIndex, i32)`, so callers map their domain type
    /// to an i32 (BOOL → 0/1, USINT/UINT → numeric, etc.).
    pub value: i32,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct WriteVariableResponse {
    pub name: String,
    pub value: i32,
}

/// Poke a variable while the program is running. Applied between scan
/// rounds (so the next round's logic sees the new value). 404 if the
/// name doesn't resolve to any declared variable; 409 if no program is
/// running.
pub async fn write_runtime_variable(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(req): Json<WriteVariableRequest>,
) -> Result<Json<WriteVariableResponse>, ApiError> {
    // Clone the handle out of the mutex so we don't hold a sync lock
    // across the .await below — see the bridge::ProgramHandle docs.
    let handle: ProgramHandle = state
        .program
        .lock()
        .expect("program")
        .as_ref()
        .cloned()
        .ok_or(ApiError::Conflict("no program running".into()))?;
    match handle.write_variable(&name, req.value).await {
        Ok(value) => Ok(Json(WriteVariableResponse { name, value })),
        Err(RuntimeWriteError::UnknownVariable(n)) => {
            Err(ApiError::NotFound(format!("variable '{n}' not declared")))
        }
        Err(RuntimeWriteError::Disconnected) => {
            Err(ApiError::Conflict("scan loop has stopped".into()))
        }
        Err(RuntimeWriteError::Vm(e)) => Err(ApiError::Internal(e)),
    }
}

// ============================================================
//  Debug control trio: pause / resume / step + force / unforce
//
//  All four endpoints share the same "look up the live handle, 409
//  if nothing running" pattern as `write_runtime_variable`. Mode
//  toggles are synchronous on the bridge side (just a mutex write);
//  force commands round-trip through the cmd channel so the scan
//  loop can validate the variable name before the ack comes back.
// ============================================================

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct StepRequest {
    /// How many scan cycles to run before re-pausing. Defaults to 1
    /// when missing — "step" without a count is the common case.
    #[serde(default = "default_step_cycles")]
    pub cycles: u32,
}

fn default_step_cycles() -> u32 {
    1
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct ForceRequest {
    pub value: i32,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ModeResponse {
    pub mode: RuntimeMode,
}

/// Look up the live program handle or return 409. Used by every
/// debug-control endpoint below.
fn live_program(state: &AppState) -> Result<ProgramHandle, ApiError> {
    state
        .program
        .lock()
        .expect("program")
        .as_ref()
        .cloned()
        .ok_or(ApiError::Conflict("no program running".into()))
}

pub async fn runtime_pause(State(state): State<AppState>) -> Result<Json<ModeResponse>, ApiError> {
    let handle = live_program(&state)?;
    handle.pause();
    Ok(Json(ModeResponse { mode: handle.mode() }))
}

pub async fn runtime_resume(State(state): State<AppState>) -> Result<Json<ModeResponse>, ApiError> {
    let handle = live_program(&state)?;
    handle.resume();
    Ok(Json(ModeResponse { mode: handle.mode() }))
}

pub async fn runtime_step(
    State(state): State<AppState>,
    body: Option<Json<StepRequest>>,
) -> Result<Json<ModeResponse>, ApiError> {
    let handle = live_program(&state)?;
    let cycles = body.map(|Json(r)| r.cycles).unwrap_or(1);
    handle.step(cycles);
    Ok(Json(ModeResponse { mode: handle.mode() }))
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ForceResponse {
    pub name: String,
    pub value: i32,
}

/// Pin a variable's value. The scan loop applies it on every cycle
/// until `unforce_runtime_variable` is called. 404 if the variable
/// isn't declared in this POU; 409 if nothing's running.
pub async fn force_runtime_variable(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(req): Json<ForceRequest>,
) -> Result<Json<ForceResponse>, ApiError> {
    let handle = live_program(&state)?;
    match handle.force_variable(&name, req.value).await {
        Ok(value) => Ok(Json(ForceResponse { name, value })),
        Err(RuntimeWriteError::UnknownVariable(n)) => {
            Err(ApiError::NotFound(format!("variable '{n}' not declared")))
        }
        Err(RuntimeWriteError::Disconnected) => {
            Err(ApiError::Conflict("scan loop has stopped".into()))
        }
        Err(RuntimeWriteError::Vm(e)) => Err(ApiError::Internal(e)),
    }
}

/// Release a forced variable. No-op (200) if the variable wasn't
/// forced — convenient for idempotent agent retries.
pub async fn unforce_runtime_variable(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let handle = live_program(&state)?;
    handle
        .unforce_variable(&name)
        .await
        .map_err(|e| match e {
            RuntimeWriteError::Disconnected => {
                ApiError::Conflict("scan loop has stopped".into())
            }
            other => ApiError::Internal(format!("{other:?}")),
        })?;
    Ok(Json(serde_json::json!({ "name": name, "forced": false })))
}

/// List currently-forced variables. Returns `[]` when nothing's
/// running (rather than 409) — easier for clients to render.
pub async fn list_runtime_forces(
    State(state): State<AppState>,
) -> Json<Vec<ForceEntry>> {
    let forces = state
        .program
        .lock()
        .expect("program")
        .as_ref()
        .map(|h| {
            h.forces()
                .into_iter()
                .map(|(name, value)| ForceEntry { name, value })
                .collect()
        })
        .unwrap_or_default();
    Json(forces)
}

// ============================================================
//  Folder deletion (devices / edges) — POU folder delete lives with
//  the rest of the POU handlers above.
// ============================================================

pub async fn delete_device_folder(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.delete_device_folder(&path).map_err(Into::into)
    })?;
    state.emit_mutation(
        topic::DEVICES,
        MutationDetail::DeviceFolderDeleted { path },
    );
    Ok(Json(RunResponse { ok: true }))
}

pub async fn delete_edge_folder(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<RunResponse>, ApiError> {
    with_project(&state, |store| {
        store.delete_edge_folder(&path).map_err(Into::into)
    })?;
    state.emit_mutation(
        topic::EDGES,
        MutationDetail::EdgeFolderDeleted { path },
    );
    Ok(Json(RunResponse { ok: true }))
}

// ============================================================
//  Demo-slave poke — inject input signals
// ============================================================

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct DemoSlavePoke {
    /// For coil / discrete_input, value is interpreted as boolean
    /// (non-zero = TRUE); for holding_register / input_register it's
    /// truncated to u16.
    pub value: i32,
}

/// Write a single address in the in-process demo Modbus slave. Useful
/// for simulating input signals during agent-driven testing — e.g.,
/// flip a discrete_input to test a fault path without driving Modbus
/// from real hardware. `kind` matches `ModbusChannelKind`.
pub async fn poke_demo_slave(
    State(state): State<AppState>,
    AxumPath((kind, addr)): AxumPath<(String, u16)>,
    Json(req): Json<DemoSlavePoke>,
) -> Result<Json<RunResponse>, ApiError> {
    let addr = addr as usize;
    // Bind each Arc<Mutex<...>> to a named local so the temporary lives
    // through the .lock() borrow — otherwise the returned Arc is dropped
    // at the end of the same statement.
    match kind.as_str() {
        "coil" => {
            let arc = state.demo_slave.coils();
            let mut guard = arc.lock().unwrap();
            *guard
                .get_mut(addr)
                .ok_or_else(|| ApiError::BadRequest("address out of range".into()))? =
                req.value != 0;
        }
        "discrete_input" => {
            let arc = state.demo_slave.discrete_inputs();
            let mut guard = arc.lock().unwrap();
            *guard
                .get_mut(addr)
                .ok_or_else(|| ApiError::BadRequest("address out of range".into()))? =
                req.value != 0;
        }
        "holding_register" => {
            let arc = state.demo_slave.holding_registers();
            let mut guard = arc.lock().unwrap();
            *guard
                .get_mut(addr)
                .ok_or_else(|| ApiError::BadRequest("address out of range".into()))? =
                req.value as u16;
        }
        "input_register" => {
            let arc = state.demo_slave.input_registers();
            let mut guard = arc.lock().unwrap();
            *guard
                .get_mut(addr)
                .ok_or_else(|| ApiError::BadRequest("address out of range".into()))? =
                req.value as u16;
        }
        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown kind '{other}' — use coil / discrete_input / holding_register / input_register"
            )));
        }
    }
    Ok(Json(RunResponse { ok: true }))
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

/// Build the ad-hoc `Tasks` for "run just this PROGRAM once":
/// one 100-ms / priority-1 task hosting one instance of the named PROGRAM.
/// Used by the /api/run path's `program` override.
fn single_program_tasks(program_name: &str) -> Tasks {
    Tasks {
        tasks: vec![Task {
            name: "plc_task".into(),
            interval_ms: 100,
            priority: 1,
        }],
        programs: vec![ProgramInstance {
            instance: format!("{program_name}_inst"),
            program: program_name.into(),
            task: "plc_task".into(),
        }],
    }
}
