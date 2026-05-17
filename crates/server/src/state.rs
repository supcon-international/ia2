use std::sync::{Arc, Mutex};
use std::time::Instant;

use iomap_modbus::DemoSlave;
use ironplc_bridge::{ProgramHandle, VarSnapshot};
use project::ProjectStore;
use tokio::sync::broadcast;

use crate::edges::AttachmentRegistry;
use crate::events::{AgentActivity, AppEvent, MutationDetail, MutationEvent};

/// Tracks the most recent agent (typically `cs` CLI) heartbeat. The
/// frontend's takeover overlay reads this — see the `agent_watchdog`
/// task in main.rs for the broadcast loop.
#[derive(Debug, Default)]
pub struct AgentActivityState {
    /// `None` until at least one heartbeat is received. Holds the
    /// latest heartbeat time after that.
    pub last_heartbeat: Option<Instant>,
    /// What the agent identified itself as ("pou create", "runtime
    /// force", etc.). Surfaced in the IDE banner.
    pub command: Option<String>,
    /// Stable per-CLI-run identifier (a UUID generated at `cs`
    /// startup). Lets us tell "one agent running fast" apart from
    /// "many agents".
    pub session: Option<String>,
    /// The current public flag — `true` after a heartbeat and until
    /// `agent_watchdog` ages it out past the TTL. Stored so we only
    /// emit AgentActivity events on edges, not every tick.
    pub active: bool,
}

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
    /// Heartbeat tracking for the "agent is in control" IDE overlay.
    /// Updated by `POST /api/agent/heartbeat`; aged out by the
    /// background watchdog task in main.rs.
    pub agent: Arc<Mutex<AgentActivityState>>,
}

/// Same shape the frontend uses, on the server side, so /api/runtime/status
/// can report it back across the wire (via `RuntimeStatus.running_info`).
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunningInfo {
    /// `compile_isolated_source` path: one PROGRAM from one .st file.
    Isolated { program: String, file_path: String },
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
            agent: Arc::new(Mutex::new(AgentActivityState::default())),
        }
    }

    /// Stamp a heartbeat from an agent client. Flips the public
    /// `active` flag (and emits an SSE) on the leading edge; the
    /// trailing edge is driven by the watchdog task that ages out
    /// stale heartbeats.
    pub fn record_agent_heartbeat(&self, command: Option<String>, session: Option<String>) {
        let edge = {
            let mut s = self.agent.lock().expect("agent mutex");
            let was_active = s.active;
            s.last_heartbeat = Some(Instant::now());
            s.command = command.clone();
            s.session = session.clone();
            s.active = true;
            !was_active
        };
        if edge {
            let _ = self.event_tx.send(AppEvent::AgentActivity(AgentActivity {
                active: true,
                command,
                session,
                since_ms: 0,
            }));
        }
    }

    /// Fire-and-forget mutation notification. Called from every CRUD
    /// handler after the on-disk write succeeds. `topic` is what the
    /// frontend's invalidationBus matches against; `detail` carries
    /// the type-tagged "what specifically changed" so the toast /
    /// auto-jump layer has context.
    ///
    /// We ignore send errors on purpose: if no SSE subscriber is
    /// listening, the broadcast channel returns `Err(NoSubscribers)`
    /// and we move on. Mutations are advisory — the next refetch
    /// will reconcile.
    pub fn emit_mutation(&self, topic: impl Into<String>, detail: MutationDetail) {
        let _ = self.event_tx.send(AppEvent::Mutation(MutationEvent {
            topic: topic.into(),
            detail,
        }));
    }
}
