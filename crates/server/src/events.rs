use ironplc_bridge::VarSnapshot;
use serde::Serialize;
use ts_rs::TS;

/// Server-pushed event delivered over SSE (`GET /api/events`).
///
/// Wire form is adjacently tagged JSON, e.g.
/// `{"type":"snapshot","data":{...VarSnapshot...}}` or `{"type":"started"}`.
///
/// Two roles share this stream:
///   1. Runtime telemetry: `Snapshot` / `Started` / `Stopped` / `Error`.
///      High-frequency, ephemeral, drives the Monitor pane.
///   2. Structural mutations: `Mutation` carries `{topic, detail}` and
///      drives cache invalidation + the toast surface in the IDE.
///
/// Two roles on one stream because the cost (single broadcast channel +
/// single SSE connection) is small and consumers filter trivially on
/// `type`. Splitting only becomes worth it if structural mutations grow
/// into something that subscribers want to receive without the runtime
/// firehose attached.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[allow(dead_code)]
pub enum AppEvent {
    Snapshot(VarSnapshot),
    Started,
    Stopped,
    Error(String),
    /// Something on disk or in AppState changed. Frontend reads
    /// `topic` to bust any matching cache key, and `detail` to drive
    /// toasts / focus heuristics ("Agent just created motor.st →
    /// auto-jump if editor is in an empty state").
    Mutation(MutationEvent),
    /// An external client (typically the `cs` CLI driven by an agent)
    /// is actively driving the server. Used by the IDE to render a
    /// "takeover" overlay so the human user knows the agent is in
    /// control and doesn't fight it for state. Goes back to `active
    /// = false` after a few seconds of no heartbeat.
    AgentActivity(AgentActivity),
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct AgentActivity {
    /// `true` while at least one agent session has heartbeat-pinged
    /// within the TTL window (default 3 s). Drops to `false` after
    /// the last heartbeat ages out.
    pub active: bool,
    /// Most recent command label the agent identified itself with —
    /// usually the `cs` subcommand name (e.g. `"pou create"`). Used
    /// in the IDE banner so the user can see *what* the agent is
    /// doing right now, not just *that* something's happening.
    pub command: Option<String>,
    /// Stable per-CLI-run identifier. Lets the frontend tell apart
    /// "one agent doing many commands fast" from "multiple agents
    /// piling on". Best-effort; the CLI generates a fresh UUID at
    /// process start.
    pub session: Option<String>,
    /// Milliseconds since the last heartbeat at the time this event
    /// fired. Lets the frontend show "agent active 0.3 s ago" instead
    /// of just a binary indicator.
    pub since_ms: u64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MutationEvent {
    /// Cache-bust key. Stable, lowercase, optionally namespaced with
    /// `:` for per-resource granularity (e.g. `pou:main`). The
    /// frontend's `invalidationBus` matches subscribers on this
    /// string verbatim.
    pub topic: String,
    /// Type-tagged details about WHAT changed. Frontend reads this to
    /// surface user-facing notifications and to decide whether to
    /// reposition the editor.
    pub detail: MutationDetail,
}

/// Type-tagged mutation summary. We pair this with `topic` rather
/// than emit a separate enum-per-resource because the frontend has
/// exactly one bus that fans out on topic; the detail is only read
/// when the toast / focus layer wants context.
///
/// Discriminator is `kind`, e.g. `{"kind":"pou_created","path":"foo"}`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MutationDetail {
    PouCreated { path: String },
    PouUpdated { path: String },
    PouDeleted { path: String },
    PouFolderCreated { path: String },
    PouFolderDeleted { path: String },
    DeviceUpserted { name: String },
    DeviceDeleted { name: String },
    DeviceFolderCreated { path: String },
    DeviceFolderDeleted { path: String },
    EdgeUpserted { name: String },
    EdgeDeleted { name: String },
    EdgeFolderCreated { path: String },
    EdgeFolderDeleted { path: String },
    EdgeAttached { name: String, local_port: u16 },
    EdgeDetached { name: String },
    IoMapChanged,
    TasksChanged,
    TasksMigrated,
    ProjectOpened { name: String, path: String },
    ProjectClosed,
    ProjectCreated { name: String, path: String },
}

/// Canonical topic strings. Centralised so the route handlers and
/// the frontend subscribers can't drift on spelling.
pub mod topic {
    /// Anything that affects the project tree's overall shape (POUs
    /// added/removed, folders, project open/close). Subscribers:
    /// the left-rail ProjectTree.
    pub const PROJECT: &str = "project";
    /// Per-POU content change. Use `format!("pou:{path}")`. Subscribers:
    /// the editor for that specific file.
    pub const PROJECT_META: &str = "project_meta";
    pub const DEVICES: &str = "devices";
    pub const EDGES: &str = "edges";
    pub const IOMAP: &str = "iomap";
    pub const TASKS: &str = "tasks";

    /// Build a per-POU topic key (e.g. `pou:main`).
    pub fn pou(path: &str) -> String {
        format!("pou:{path}")
    }
    /// Build a per-device topic key.
    pub fn device(name: &str) -> String {
        format!("device:{name}")
    }
    /// Build a per-edge topic key.
    pub fn edge(name: &str) -> String {
        format!("edge:{name}")
    }
}
