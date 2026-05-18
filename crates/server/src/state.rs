use std::sync::{Arc, Mutex};
use std::time::Instant;

use iomap_modbus::DemoSlave;
use ironplc_bridge::{ProgramHandle, VarSnapshot};
use project::ProjectStore;
use tokio::sync::broadcast;

use crate::edges::AttachmentRegistry;
use crate::events::{AgentActivity, AppEvent, MutationDetail, MutationEvent};

/// Holds every project the server has open, plus a pointer to the
/// "active" one. The active project is the implicit target for
/// requests that don't specify a project name in the `X-IA2-Project`
/// header — i.e. every existing CLI / single-window IDE workflow.
///
/// Order in `open` is insertion order so the IDE's project picker can
/// present them stably. Active is by name (not index) so removing a
/// project doesn't dangle the pointer.
///
/// The collection is small in practice — a user has maybe 1-3
/// projects open at a time — so linear scans on `Vec` beat the cache
/// allocations of a `HashMap`. `find_by_name` is linear and the
/// length is the number of windows the user is staring at, not a
/// database.
#[derive(Default)]
pub struct ProjectRegistry {
    open: Vec<ProjectStore>,
    active: Option<String>,
}

impl ProjectRegistry {
    /// Look up a project by name. Returns `None` if no such project
    /// is open in this server.
    pub fn get(&self, name: &str) -> Option<&ProjectStore> {
        self.open.iter().find(|p| p.name() == name)
    }

    /// Currently-active project (the default target when an HTTP
    /// request doesn't specify one). `None` until at least one
    /// project is opened.
    pub fn active(&self) -> Option<&ProjectStore> {
        self.active.as_deref().and_then(|n| self.get(n))
    }

    /// Name of the active project, if any.
    pub fn active_name(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Insert a project, replacing any existing entry of the same
    /// name (re-open is idempotent), and mark it active. Inserting
    /// the same project a second time keeps its slot in `open` —
    /// the IDE's picker order is stable across re-opens.
    pub fn insert_and_activate(&mut self, store: ProjectStore) {
        let name = store.name().to_string();
        if let Some(slot) = self.open.iter_mut().find(|p| p.name() == name.as_str()) {
            *slot = store;
        } else {
            self.open.push(store);
        }
        self.active = Some(name);
    }

    /// Mark an already-open project as active. No-op if name isn't
    /// open. Used by the routes that take an `X-IA2-Project` header
    /// — touching a project promotes it (LRU-ish behaviour).
    pub fn set_active(&mut self, name: &str) -> bool {
        if self.open.iter().any(|p| p.name() == name) {
            self.active = Some(name.to_string());
            true
        } else {
            false
        }
    }

    /// Remove a project. Returns `true` if it was open. If the closed
    /// project was active, the most-recently-inserted remaining
    /// project becomes active (or `None` if the set is now empty).
    pub fn remove(&mut self, name: &str) -> bool {
        let initial_len = self.open.len();
        self.open.retain(|p| p.name() != name);
        let removed = self.open.len() != initial_len;
        if removed && self.active.as_deref() == Some(name) {
            self.active = self.open.last().map(|p| p.name().to_string());
        }
        removed
    }

    /// Snapshot of currently-open projects, in insertion order, for
    /// the `GET /api/projects` endpoint and persistence on shutdown.
    pub fn iter(&self) -> impl Iterator<Item = &ProjectStore> {
        self.open.iter()
    }

    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }
}

/// Tracks agent activity — the source of truth for the IDE's
/// "agent in control" takeover overlay. Two distinct shapes:
///
///   1. **Session** (`AgentSession`) — an *explicit* enter / leave
///      pair around a coherent stretch of work. The agent decides
///      when control starts (e.g. "rebuilding tank controller") and
///      when it ends. While the session is open the overlay stays
///      ON; individual heartbeats only matter for crash detection.
///      This is the recommended path for any multi-step agent
///      workflow.
///
///   2. **Transient heartbeats** — a single `cs` command that posts
///      to `/api/agent/heartbeat` without holding a session. The
///      overlay flashes on, then ages out after `TRANSIENT_TTL`.
///      Kept for back-compat with simple one-off CLI calls (and as
///      the underlying mechanism for session crash recovery).
///
/// The `active` field is the union — true if a session is open OR a
/// recent transient heartbeat hasn't aged out. The watchdog task in
/// `main.rs` is responsible for clearing both and re-emitting
/// AgentActivity SSE on the trailing edge.
#[derive(Debug, Default)]
pub struct AgentActivityState {
    /// `None` until at least one heartbeat is received. Holds the
    /// latest heartbeat time after that.
    pub last_heartbeat: Option<Instant>,
    /// What the agent identified itself as ("pou create", "runtime
    /// force", etc.). Surfaced in the IDE banner when no session is
    /// active.
    pub command: Option<String>,
    /// Stable per-CLI-run identifier sent on individual heartbeats —
    /// distinct from the session id below. Lets us tell "one agent
    /// running fast" apart from "many agents".
    pub session_hint: Option<String>,
    /// Long-running session, if the agent opened one with
    /// `/api/agent/session/start`. None when no session is active.
    pub session: Option<AgentSession>,
    /// Public flag — true when EITHER `session` is Some OR a
    /// transient heartbeat is still inside its TTL. Mirrored as a
    /// field (not recomputed each read) so the watchdog can emit
    /// edge-transition SSE events without comparing against the
    /// previous tick.
    pub active: bool,
}

/// One open agent takeover session. Lifetime: `POST /api/agent/session/start`
/// → server creates this; `POST /api/agent/session/end { id }` (or the
/// watchdog detecting no-heartbeat-for-too-long) → drops it.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export)]
pub struct AgentSession {
    /// Client-generated unique id. Used by `end` to confirm the
    /// caller owns the session it's ending (so a stale `cs agent
    /// leave` from an old terminal can't kick a fresh agent).
    pub id: String,
    /// Human-readable label for the IDE banner ("rebuilding tank
    /// controller", "running tests", "agent: investigating leak").
    pub label: String,
    /// Microseconds since UNIX epoch — for "started 12 s ago" UI
    /// rendering. We don't use `Instant` because that's not
    /// serializable.
    pub started_us: u64,
    /// Last heartbeat we got for THIS session. Drives crash
    /// recovery: if the agent process dies, we age the session out
    /// instead of leaving the overlay stuck on forever. Skipped on
    /// the wire — the watchdog cares but the frontend doesn't.
    #[serde(skip)]
    #[ts(skip)]
    pub last_heartbeat: Instant,
}

#[derive(Clone)]
pub struct AppState {
    pub start_time: Instant,
    /// All currently-open projects, plus which one is the implicit
    /// target for requests that don't supply `X-IA2-Project`.
    pub projects: Arc<Mutex<ProjectRegistry>>,
    /// The single PROGRAM the server is currently running. Global —
    /// the hardware (Modbus, EtherCAT) can only be controlled by one
    /// PROGRAM at a time. When set, also records which project the
    /// running program belongs to so the IDE can show
    /// "running: foo's main" across windows.
    pub program: Arc<Mutex<Option<RunningProgram>>>,
    pub event_tx: broadcast::Sender<AppEvent>,
    pub demo_slave: DemoSlave,
    /// The address the in-process demo Modbus slave is listening on
    /// (e.g. "127.0.0.1:5502"). Empty string when the slave is disabled.
    pub demo_modbus_addr: String,
    /// Currently-open `ssh -N -L` tunnels to edge boxes, keyed by
    /// `(project_name, edge_name)` so two projects with the same edge
    /// name don't fight over the tunnel. Lifecycle is owned by the
    /// server process — dropping an entry kills the child via
    /// `kill_on_drop`.
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

/// Pairs the active `ProgramHandle` with the name of the project it
/// belongs to. Stored together so `/api/runtime/status` can answer
/// "what's running, and whose project does it belong to?" without an
/// extra cross-reference table.
pub struct RunningProgram {
    pub project_name: String,
    pub handle: ProgramHandle,
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
            projects: Arc::new(Mutex::new(ProjectRegistry::default())),
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

    /// Stamp a heartbeat from an agent client. If `session_id`
    /// matches the currently-open session, refresh its watchdog
    /// timer (and ignore the command label — the session label wins).
    /// Otherwise, fall into the legacy "transient" path: refresh the
    /// per-heartbeat command + age out after TRANSIENT_TTL.
    ///
    /// The leading-edge AgentActivity event fires when overall
    /// activity transitions from inactive → active; the trailing
    /// edge is the watchdog's job.
    pub fn record_agent_heartbeat(&self, command: Option<String>, session_id: Option<String>) {
        let edge = {
            let mut s = self.agent.lock().expect("agent mutex");
            let was_active = s.active;
            let now = Instant::now();
            // If a session is open and the heartbeat's session_id
            // matches, refresh the session's own watchdog. We still
            // bump `last_heartbeat` so a "session is active" view
            // and a "wire still healthy" view stay in sync.
            if let (Some(sess), Some(id)) = (s.session.as_mut(), session_id.as_deref()) {
                if sess.id == id {
                    sess.last_heartbeat = now;
                }
            }
            s.last_heartbeat = Some(now);
            s.command = command.clone();
            s.session_hint = session_id.clone();
            s.active = true;
            !was_active
        };
        if edge {
            // Snapshot the session label so the wire event carries
            // it (the frontend renders the label as the banner text).
            let label = self
                .agent
                .lock()
                .ok()
                .and_then(|s| s.session.as_ref().map(|sess| sess.label.clone()));
            let _ = self.event_tx.send(AppEvent::AgentActivity(AgentActivity {
                active: true,
                command,
                session: session_id,
                session_label: label,
                since_ms: 0,
            }));
        }
    }

    /// Open an explicit agent takeover session. The IDE overlay
    /// stays on until `end_agent_session(id)` is called (or the
    /// watchdog ages it out after SESSION_TTL of no heartbeats).
    ///
    /// Returns the session that was actually opened. If another
    /// session is already running, the policy here is **replace** —
    /// a new agent kicks the previous, broadcasting a fresh
    /// AgentActivity event with the new label. That matches the
    /// real-world usage (one human, one agent at a time on a given
    /// server); strict mutex semantics with 409 errors would be
    /// surprising when, say, a previous `cs agent run` left a
    /// session stranded.
    pub fn start_agent_session(&self, id: String, label: String) -> AgentSession {
        let session = AgentSession {
            id: id.clone(),
            label: label.clone(),
            started_us: now_unix_us(),
            last_heartbeat: Instant::now(),
        };
        {
            let mut s = self.agent.lock().expect("agent mutex");
            s.session = Some(session.clone());
            s.last_heartbeat = Some(Instant::now());
            s.active = true;
        }
        // Always emit on session start — even if `active` was already
        // true (a transient heartbeat was in flight), the label now
        // changes, so subscribers need to repaint.
        let _ = self.event_tx.send(AppEvent::AgentActivity(AgentActivity {
            active: true,
            command: None,
            session: Some(id),
            session_label: Some(label),
            since_ms: 0,
        }));
        session
    }

    /// Close the agent takeover session. If `id` is `Some(...)` we
    /// only close when it matches the active session's id —
    /// idempotent + race-safe (a stale `cs agent leave` from an old
    /// terminal won't kick a fresh agent). If `None`, force-close
    /// whatever session is open — this is the "kick" path used by
    /// the IDE's "Take over" button.
    ///
    /// Returns `true` if a session was actually ended.
    pub fn end_agent_session(&self, id: Option<&str>) -> bool {
        let closed = {
            let mut s = self.agent.lock().expect("agent mutex");
            match s.session.as_ref() {
                Some(sess) if id.is_none_or(|i| i == sess.id) => {
                    s.session = None;
                    // Wipe the heartbeat baseline too so the
                    // overlay actually disappears — without this,
                    // a still-fresh `last_heartbeat` would keep
                    // `active` true via the transient path until
                    // its TTL elapsed.
                    s.last_heartbeat = None;
                    s.command = None;
                    s.session_hint = None;
                    s.active = false;
                    true
                }
                _ => false,
            }
        };
        if closed {
            let _ = self.event_tx.send(AppEvent::AgentActivity(AgentActivity {
                active: false,
                command: None,
                session: None,
                session_label: None,
                since_ms: 0,
            }));
        }
        closed
    }

    /// Fire-and-forget mutation notification scoped to one project.
    /// Called from every CRUD handler after the on-disk write
    /// succeeds. The `project` argument is the project the mutation
    /// belongs to — frontend windows filter SSE events by their
    /// currently-displayed project so window A doesn't react to
    /// window B's POU save.
    ///
    /// `topic` is what the frontend's invalidationBus matches
    /// against; `detail` carries the type-tagged "what specifically
    /// changed" so the toast / auto-jump layer has context.
    ///
    /// We ignore send errors on purpose: if no SSE subscriber is
    /// listening, the broadcast channel returns `Err(NoSubscribers)`
    /// and we move on. Mutations are advisory — the next refetch
    /// will reconcile.
    pub fn emit_mutation(
        &self,
        project: impl Into<String>,
        topic: impl Into<String>,
        detail: MutationDetail,
    ) {
        let _ = self.event_tx.send(AppEvent::Mutation(MutationEvent {
            project: project.into(),
            topic: topic.into(),
            detail,
        }));
    }
}

/// Wall-clock microseconds since the UNIX epoch. Used for session
/// `started_us` so the frontend can render "started 12 s ago". We
/// don't use `Instant` because that's a monotonic-clock-only type
/// (no calendar time) and not serializable.
fn now_unix_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}
