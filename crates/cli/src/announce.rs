//! Best-effort heartbeat / takeover-session announcement to the IDE, so
//! the "agent in control" overlay renders whenever the CLI drives IA2.

use crate::http::http_agent;
use crate::{
    Command, DeviceCmd, EdgeCmd, HmiCmd, IomapCmd, LibraryCmd, NorthboundCmd, PouCmd, ProjectCmd,
    RuntimeCmd, TasksCmd,
};

/// Env var holding the active session id between `cs agent enter`
/// and `cs agent leave`. Set by the user (`export
/// IA2_AGENT_SESSION=$(cs agent enter --label ...)`) or by `cs
/// agent run` when it spawns the inner command.
pub(crate) const SESSION_ENV: &str = "IA2_AGENT_SESSION";

// Short-lived `cs` commands ping POST /api/agent/heartbeat at start.
// The server keeps the "agent active" flag set for ~3 s after the
// last heartbeat; the IDE renders its takeover overlay while it's
// set. Read-only commands (check, info, status, symbols, explain,
// transpile) deliberately skip the heartbeat — querying state isn't
// "operating" and shouldn't trigger the overlay.

/// Return `Some((server, label))` for commands that should announce
/// before dispatching, `None` for read-only commands. The label is
/// what shows up in the IDE banner ("Agent in control · pou create").
pub(crate) fn announce_target(cmd: &Command) -> Option<(&str, &'static str)> {
    match cmd {
        // Static analysis / self-managed — no IDE server to announce to.
        // (`project check`/`info` operate on a directory on disk.)
        Command::Check { .. }
        | Command::Transpile { .. }
        | Command::Explain { .. }
        | Command::Symbols { .. }
        | Command::Project(ProjectCmd::Check { .. })
        | Command::Project(ProjectCmd::Info { .. })
        | Command::Agent(_) => None,

        // Everything that talks to the IDE server announces — reads
        // INCLUDED — so the takeover overlay renders whenever an agent
        // drives IA2 over the HTTP API, not only on mutations. (Inside a
        // `cs agent run`/`enter` session the forwarded IA2_AGENT_SESSION
        // keeps these on the steady session banner instead of flashing.)
        Command::Project(ProjectCmd::List { server, .. }) => Some((&server.server, "project list")),
        Command::Project(ProjectCmd::Create { server, .. }) => {
            Some((&server.server, "project create"))
        }
        Command::Project(ProjectCmd::Open { server, .. }) => Some((&server.server, "project open")),
        Command::Project(ProjectCmd::Close { server, .. }) => {
            Some((&server.server, "project close"))
        }

        Command::Pou(PouCmd::Create { server, .. }) => Some((&server.server, "pou create")),
        Command::Pou(PouCmd::Save { server, .. }) => Some((&server.server, "pou save")),
        Command::Pou(PouCmd::Delete { server, .. }) => Some((&server.server, "pou delete")),

        Command::Device(DeviceCmd::List { server, .. }) => Some((&server.server, "device list")),
        Command::Device(DeviceCmd::Get { server, .. }) => Some((&server.server, "device get")),
        Command::Device(DeviceCmd::Create { server, .. }) => {
            Some((&server.server, "device create"))
        }
        Command::Device(DeviceCmd::Set { server, .. }) => Some((&server.server, "device set")),
        Command::Device(DeviceCmd::Delete { server, .. }) => {
            Some((&server.server, "device delete"))
        }
        Command::Device(DeviceCmd::EsiAssemble { server, .. }) => {
            Some((&server.server, "device esi-assemble"))
        }
        Command::Device(DeviceCmd::OpcuaBrowse { server, .. }) => {
            Some((&server.server, "device opcua-browse"))
        }

        Command::Edge(EdgeCmd::List { server, .. }) => Some((&server.server, "edge list")),
        Command::Edge(EdgeCmd::Get { server, .. }) => Some((&server.server, "edge get")),
        Command::Edge(EdgeCmd::Logs { server, .. }) => Some((&server.server, "edge logs")),
        Command::Edge(EdgeCmd::Scan { server, .. }) => Some((&server.server, "edge scan")),
        Command::Edge(EdgeCmd::System { server, .. }) => Some((&server.server, "edge system")),
        Command::Edge(EdgeCmd::Create { server, .. }) => Some((&server.server, "edge create")),
        Command::Edge(EdgeCmd::Set { server, .. }) => Some((&server.server, "edge set")),
        Command::Edge(EdgeCmd::Delete { server, .. }) => Some((&server.server, "edge delete")),

        Command::Iomap(IomapCmd::Get { server, .. }) => Some((&server.server, "iomap get")),
        Command::Iomap(IomapCmd::Set { server, .. }) => Some((&server.server, "iomap set")),

        Command::Hmi(HmiCmd::List { server }) => Some((&server.server, "hmi list")),
        Command::Hmi(HmiCmd::Get { server, .. }) => Some((&server.server, "hmi get")),
        Command::Hmi(HmiCmd::Create { server, .. }) => Some((&server.server, "hmi create")),
        Command::Hmi(HmiCmd::Save { server, .. }) => Some((&server.server, "hmi save")),
        Command::Hmi(HmiCmd::Op { server, .. }) => Some((&server.server, "hmi op")),
        Command::Hmi(HmiCmd::Check { server, .. }) => Some((&server.server, "hmi check")),
        Command::Hmi(HmiCmd::Generate { server, .. }) => Some((&server.server, "hmi generate")),
        Command::Hmi(HmiCmd::Symbols { server }) => Some((&server.server, "hmi symbols")),
        Command::Hmi(HmiCmd::Delete { server, .. }) => Some((&server.server, "hmi delete")),
        Command::Tasks(TasksCmd::Get { server, .. }) => Some((&server.server, "tasks get")),
        Command::Northbound(NorthboundCmd::Get { server, .. }) => {
            Some((&server.server, "northbound get"))
        }
        Command::Northbound(NorthboundCmd::Set { server, .. }) => {
            Some((&server.server, "northbound set"))
        }
        Command::Tasks(TasksCmd::Set { server, .. }) => Some((&server.server, "tasks set")),

        Command::Library(LibraryCmd::List { server, .. }) => Some((&server.server, "library list")),
        Command::Library(LibraryCmd::Import { server, .. }) => {
            Some((&server.server, "library import"))
        }
        Command::Library(LibraryCmd::Remove { server, .. }) => {
            Some((&server.server, "library remove"))
        }

        Command::Probe { server, .. } => Some((&server.server, "probe")),
        Command::Run { server, .. } => Some((&server.server, "run")),
        Command::Stop { server, .. } => Some((&server.server, "stop")),
        Command::Deploy { server, .. } => Some((&server.server, "deploy")),

        Command::Runtime(RuntimeCmd::Status { server, .. }) => {
            Some((&server.server, "runtime status"))
        }
        Command::Runtime(RuntimeCmd::Pause { server, .. }) => {
            Some((&server.server, "runtime pause"))
        }
        Command::Runtime(RuntimeCmd::Resume { server, .. }) => {
            Some((&server.server, "runtime resume"))
        }
        Command::Runtime(RuntimeCmd::Step { server, .. }) => Some((&server.server, "runtime step")),
        Command::Runtime(RuntimeCmd::Force { server, .. }) => {
            Some((&server.server, "runtime force"))
        }
        Command::Runtime(RuntimeCmd::Unforce { server, .. }) => {
            Some((&server.server, "runtime unforce"))
        }
        Command::Runtime(RuntimeCmd::Write { server, .. }) => {
            Some((&server.server, "runtime write"))
        }
    }
}

/// Per-process session id. Generated lazily so commands that don't
/// announce don't pay the cost. Format: `cs-<pid>-<nanos>` — random
/// enough for "tell agents apart" without pulling the uuid crate.
pub(crate) fn session_id() -> &'static str {
    use std::sync::OnceLock;
    static SESSION: OnceLock<String> = OnceLock::new();
    SESSION.get_or_init(|| {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("cs-{pid}-{nanos:x}")
    })
}

/// Fire-and-forget heartbeat. Short timeout because we'd rather miss
/// the visual cue than hold up a command's actual work.
///
/// Session attribution: if the caller is inside a `cs agent run`
/// wrapper (or a manually-`enter`ed session), the parent's session
/// id lives in `IA2_AGENT_SESSION`. We forward it so the server's
/// session-watchdog refreshes the right session instead of starting
/// a competing transient heartbeat that would race the overlay's
/// label back and forth.
pub(crate) fn announce_agent(server: &str, command_label: &str) {
    let session = std::env::var(SESSION_ENV)
        .ok()
        .unwrap_or_else(|| session_id().to_string());
    let _ = http_agent()
        .post(&format!("{server}/api/agent/heartbeat"))
        .timeout(std::time::Duration::from_millis(300))
        .send_json(serde_json::json!({
            "command": command_label,
            "session": session,
        }));
}
