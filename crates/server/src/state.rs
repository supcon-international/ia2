use std::sync::{Arc, Mutex};
use std::time::Instant;

use iomap_modbus::DemoSlave;
use ironplc_bridge::{ProgramHandle, VarSnapshot};
use project::ProjectStore;
use tokio::sync::broadcast;

use crate::edges::AttachmentRegistry;
use crate::events::AppEvent;

#[derive(Clone)]
pub struct AppState {
    pub start_time: Instant,
    pub project: Arc<Mutex<Option<ProjectStore>>>,
    pub program: Arc<Mutex<Option<ProgramHandle>>>,
    pub event_tx: broadcast::Sender<AppEvent>,
    pub demo_slave: DemoSlave,
    /// The address the in-process demo Modbus slave is listening on
    /// (e.g. "127.0.0.1:5502"). Empty string when the slave is disabled.
    pub demo_modbus_addr: String,
    /// Currently-open `ssh -N -L` tunnels to edge boxes, keyed by edge
    /// name. Lifecycle is owned by the server process — dropping an
    /// entry kills the child via `kill_on_drop`.
    pub attachments: Arc<AttachmentRegistry>,
    /// Most recent `VarSnapshot` from the running bridge. Updated by the
    /// SSE forwarder task; persists across stop so the Monitor pane (and
    /// debug agents) can read the last-known state after the program
    /// ends. Cleared on close-project.
    pub last_snapshot: Arc<Mutex<Option<VarSnapshot>>>,
    /// Last bridge / runtime error surfaced to /api/runtime/status, or
    /// `None` if the last run is clean. Updated when AppEvent::Error
    /// fires and on a clean Started.
    pub last_error: Arc<Mutex<Option<String>>>,
    /// What the most-recent /api/run call asked the bridge to run.
    /// Lets the IDE recover "running ad-hoc / running scheduled, which
    /// PROGRAM(s)" after a page reload without an out-of-band channel.
    /// Cleared on /api/stop and on close-project.
    pub running_info: Arc<Mutex<Option<RunningInfo>>>,
}

/// Same shape the frontend uses, on the server side, so /api/runtime/status
/// can report it back across the wire (via `RuntimeStatus.running_info`).
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunningInfo {
    /// `compile_isolated_source` path: one PROGRAM from one .st file.
    Isolated {
        program: String,
        file_path: String,
    },
    /// `compile_project_with_tasks` (or `compile_project`): full
    /// tasks.toml schedule. Programs are the PROGRAM names, not the
    /// instance names — that's what makes sense to a human at a glance.
    Scheduled { programs: Vec<String> },
}

impl AppState {
    pub fn new(demo_slave: DemoSlave, demo_modbus_addr: String) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            start_time: Instant::now(),
            project: Arc::new(Mutex::new(None)),
            program: Arc::new(Mutex::new(None)),
            event_tx,
            demo_slave,
            demo_modbus_addr,
            attachments: AttachmentRegistry::new(),
            last_snapshot: Arc::new(Mutex::new(None)),
            last_error: Arc::new(Mutex::new(None)),
            running_info: Arc::new(Mutex::new(None)),
        }
    }
}
