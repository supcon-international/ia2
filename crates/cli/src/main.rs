//! `cs` — agent-first command-line interface for IA2.
//!
//! Every workflow that an engineer or agent would do **before** the
//! runtime starts is wired here: validate, transpile, compile, inspect.
//! Online operations (live values, attach to edge, Run/Stop) stay on
//! HTTP because they need the running server.
//!
//! Design notes — see also `MEMORY/principles.md` § "CLI is the
//! headline agent interface":
//!
//! - Every subcommand supports `--json` for machine output. Human
//!   pretty-printing is the default for plain runs.
//! - Exit codes follow Unix convention:
//!   * 0 — clean success
//!   * 1 — ran fine but found errors in the user's source (diagnostics)
//!   * 2 — usage error (bad arguments, missing file)
//!   * >2 — infrastructure failure (I/O, bridge crash)
//! - Help text is written FOR THE AGENT — say when to use the tool,
//!   when NOT to, what to call next. Style reference:
//!   `vendor/ironplc/compiler/mcp/src/server.rs`.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod announce;
mod cmd;
mod http;

use crate::announce::{announce_agent, announce_target};
use crate::cmd::agent::cmd_agent;
use crate::cmd::analysis::{cmd_check, cmd_explain, cmd_symbols, cmd_transpile};
use crate::cmd::config::{cmd_iomap, cmd_library, cmd_northbound, cmd_tasks};
use crate::cmd::device::cmd_device;
use crate::cmd::edge::{cmd_deploy, cmd_edge, cmd_probe};
use crate::cmd::pou::cmd_pou;
use crate::cmd::project::{
    cmd_project_check, cmd_project_close, cmd_project_create, cmd_project_info, cmd_project_list,
    cmd_project_open,
};
use crate::cmd::runtime::{cmd_run, cmd_runtime, cmd_stop};
use crate::http::{ServerOpt, PROJECT_OVERRIDE};

// =================================================================
//   Top-level command surface
// =================================================================

/// `cs` — IA2 CLI. Static analysis, transpile, project inspection.
/// Online runtime operations stay on the HTTP API. The binary is
/// called `cs` (two letters, no shift, no digit) rather than `ia2`
/// for shell ergonomics; rename via shell alias if you'd rather
/// match the product name.
#[derive(Parser, Debug)]
#[command(
    name = "cs",
    version,
    about = "IA2 CLI — agent-first static analysis & project tools",
    long_about = "\
IA2 CLI (`cs`) — agent-first static analysis & project tools.

When to use this binary:
  - Before runtime starts: validate, transpile, compile, inspect.
  - In CI / pre-commit / batch refactor scripts.

When NOT to use this binary:
  - Live values, attach to a running edge, Run/Stop control — those
    require the HTTP / SSE server (`cargo run -p server`).

Every subcommand returns:
  exit 0  → success
  exit 1  → clean run but the source has errors (squiggle territory)
  exit 2  → usage error
  exit ≥3 → infrastructure failure

Most subcommands take `--json` to switch from human pretty-print to
machine-readable JSON on stdout.
"
)]
struct Cli {
    /// Target a specific open project on a multi-project server. When
    /// absent (the default for single-project setups), the CLI lets
    /// the server pick its "active project" — i.e. whichever project
    /// was most recently opened. Multi-window IDE users pass this to
    /// target a specific workbench window's project. Wired up as a
    /// top-level flag so every subcommand inherits it without repeating
    /// the option per command.
    #[arg(long, global = true)]
    project: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Validate a POU file (ST or LD). Primary tool for the
    /// edit-validate-fix loop.
    ///
    /// Returns the same diagnostics as `POST /api/check`. Auto-detects
    /// language from the file extension (`.st` → ST, `.ld.json` → LD).
    /// Multiple files are checked TOGETHER: each file sees the others'
    /// declarations, so `cs check pous/*.st` resolves FUNCTION_BLOCKs
    /// declared in sibling files exactly like a project compile. Exit
    /// code is 1 if any file has errors.
    ///
    /// Use this in CI, pre-commit hooks, or any time an agent wants to
    /// confirm a change is well-formed before invoking `compile` or
    /// reporting success to the human. Cheap (no codegen) — call it
    /// liberally.
    ///
    /// `--explain` adds the full RST explanation from ironplc's problem
    /// documentation under each diagnostic — useful when you don't
    /// recognise the error code. (`--json` always includes the
    /// explanation in the payload; `--explain` only affects human
    /// output.)
    #[command(verbatim_doc_comment)]
    Check {
        /// POU file(s) — `.st` or `.ld.json`.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Output JSON diagnostics on stdout (one array, all files).
        #[arg(long)]
        json: bool,
        /// In human mode, append each diagnostic's full RST
        /// explanation. Ignored in `--json` mode (explanation is
        /// always present in the JSON payload as the `explanation`
        /// field).
        #[arg(long)]
        explain: bool,
    },

    /// Show the Structured Text a graphical POU compiles to.
    ///
    /// LD (and future FBD / SFC) get lowered to ST before reaching
    /// ironplc. This subcommand prints that intermediate ST so an
    /// agent can read the actual code the compiler sees. Useful for
    /// understanding why an `ld_location` diagnostic points where it
    /// does, or for spot-checking that a transpiler change produced
    /// the expected output.
    ///
    /// `--with-map` additionally emits the line-resolution source map
    /// as JSON (one entry per ST line: which LD element it came from).
    /// Only meaningful for graphical POUs.
    #[command(verbatim_doc_comment)]
    Transpile {
        /// LD JSON file (`.ld.json`). ST files transpile to themselves.
        file: PathBuf,
        /// Also emit the source map (line → LD element) as JSON.
        /// Output becomes `{ "st": "...", "source_map": [...] }`.
        #[arg(long)]
        with_map: bool,
    },

    /// Project-level operations. Operate on a project directory (the
    /// one containing `project.toml`), not individual files.
    #[command(subcommand)]
    Project(ProjectCmd),

    /// CRUD on POU files in the open project — wraps the HTTP API so
    /// agents don't have to hand-roll JSON requests and don't have to
    /// remember the `language` filename convention.
    ///
    /// All subcommands require a server with an open project; use
    /// `cs project open` first if nothing's loaded.
    #[command(subcommand)]
    Pou(PouCmd),

    /// Start a compiled project / program on the running server.
    ///
    /// Three flavours, mirroring the IDE's Run buttons:
    ///   * `cs run`                       — schedule everything in tasks.toml
    ///   * `cs run --program NAME`        — pick one PROGRAM by name from the project
    ///   * `cs run --program NAME --file PATH` — isolated run of a stand-alone .st file
    ///
    /// Returns when the runtime accepts the command. Watch live values with
    /// `cs runtime status` or `curl /api/runtime/snapshot`.
    #[command(verbatim_doc_comment)]
    Run {
        /// PROGRAM name to run (must be in tasks.toml or in `--file`).
        #[arg(long)]
        program: Option<String>,
        /// File path for an isolated, off-task run. Requires `--program`.
        #[arg(long)]
        file: Option<PathBuf>,
        #[command(flatten)]
        server: ServerOpt,
    },

    /// Stop the running runtime. No-op if nothing is running.
    Stop {
        #[command(flatten)]
        server: ServerOpt,
    },

    /// Print the full RST documentation for an ironplc problem code.
    ///
    /// Looks up `P0001` / `P4007` / `P9001` etc. in ironplc's embedded
    /// problem registry — same source `cs check --explain` pulls from
    /// when it appends explanations to diagnostics. Useful when an
    /// agent has a code but no diagnostic context yet, or when a human
    /// wants to read the full RST page.
    ///
    /// Exit code: 0 if the code exists, 1 if it doesn't.
    #[command(verbatim_doc_comment)]
    Explain {
        /// Problem code (case-sensitive; ironplc uses upper-case `P`).
        code: String,
    },

    /// Runtime debug commands: pause / resume / step / force / unforce
    /// against a running server. These talk to `localhost:3001` (or
    /// `--server`) over HTTP — agents and humans share the same
    /// surface as the GUI's debug controls.
    #[command(subcommand)]
    Runtime(RuntimeCmd),

    /// Push the open project to a configured edge over SSH.
    ///
    /// Mirrors the IDE's Edge pane "Deploy" button. The server tars the
    /// current project, streams it to the edge over `ssh`, extracts to
    /// a versioned directory, atomically flips the `current` symlink,
    /// and restarts the systemd unit. Old versions are kept (rollback
    /// = swap the symlink again).
    ///
    /// `name` is the edge entry in the project (visible in the Edge
    /// pane and `project.toml`). Requires a server with the project
    /// open — same model as `cs run`. For CI/CD: start a headless
    /// server pointed at the project, then `cs deploy <name>`.
    ///
    /// Returns the assigned version timestamp and the full deploy log.
    /// Exit code: 0 on success, 1 on remote failure (script ran but
    /// exited non-zero), 3 on local error (no project, bad edge name).
    #[command(verbatim_doc_comment)]
    Deploy {
        /// Edge name (entry in the open project's edge list).
        name: String,
        /// JSON output on stdout (deploy report).
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        server: ServerOpt,
    },

    /// Probe a configured edge — quick SSH+curl reachability check.
    ///
    /// Same as the IDE's Edge pane status badge. Returns the
    /// `EdgeProbe` shape: `reachable`, `scan_count`, `uptime_secs`,
    /// `runtime_version`. Exit code: 0 if reachable, 1 if not.
    #[command(verbatim_doc_comment)]
    Probe {
        /// Edge name (entry in the open project's edge list).
        name: String,
        /// JSON output on stdout.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        server: ServerOpt,
    },

    /// CRUD on devices in the open project. Mirrors the IDE's
    /// Device pane: create, list, get full config, replace from a
    /// JSON file, delete. The complex shapes (Modbus channels,
    /// EtherCAT PDO maps) are easiest to manage via `set --from
    /// file.json` after editing a config snapshot.
    #[command(subcommand)]
    Device(DeviceCmd),

    /// CRUD on edges (deploy targets) in the open project. Same
    /// shape as `cs device` but for SSH-reachable runtime hosts.
    /// Edge deploy / probe live as top-level commands (`cs deploy`
    /// / `cs probe`).
    #[command(subcommand)]
    Edge(EdgeCmd),

    /// Read or write the project's IoMap (variable → device.channel
    /// bindings the scan loop uses).
    #[command(subcommand)]
    Iomap(IomapCmd),

    /// Read or write the project's task schedule (tasks.toml).
    #[command(subcommand)]
    Tasks(TasksCmd),

    /// Author operator screens (HMI) under the project's `hmi/`.
    ///
    /// The intended agent workflow is INCREMENTAL: `cs hmi generate`
    /// lays a deterministic baseline from the project's variables, then
    /// you reshape it one element at a time with `cs hmi op` — every op
    /// renders live (with a spawn animation) in any open IDE canvas, so
    /// generate → look → op → look. `cs hmi symbols` prints the palette
    /// contract; `cs hmi check` validates structure + variable names.
    #[command(subcommand)]
    Hmi(HmiCmd),

    /// Read or write the project's northbound publishing config
    /// (northbound.toml — MQTT to supOS/Tier0; applied by the edge
    /// runtime on its next restart/deploy).
    #[command(subcommand)]
    Northbound(NorthboundCmd),

    /// Manage first-class FB libraries vendored into the project
    /// (`pous/lib/<name>/`). Mirrors the IDE's "Import library blocks"
    /// flow so an agent can browse the registry, pull blocks in, and
    /// drop them again without the GUI. Requires a server with an open
    /// project.
    #[command(subcommand)]
    Library(LibraryCmd),

    /// Take a long-running agent session around a wrapped command.
    ///
    /// The single most important command for any multi-step agent
    /// workflow. Server-side, this opens an explicit takeover
    /// session before running the inner command and closes it
    /// afterwards — the IDE banner stays on with `--label` text the
    /// whole time, instead of flickering between every `cs` call.
    /// A background heartbeat thread refreshes the session every
    /// second so the watchdog never thinks the agent crashed.
    ///
    /// Pattern:
    ///   `cs agent run --label "rebuilding tank" -- bash -c '...'`
    ///   `cs agent run --label "tests" -- pytest`
    ///
    /// If the inner command exits non-zero, the session is still
    /// closed cleanly (try/finally semantics). Ctrl-C closes the
    /// session before propagating SIGINT to the inner command.
    #[command(verbatim_doc_comment)]
    #[command(subcommand)]
    Agent(AgentCmd),

    /// List the symbols declared in a POU — variables, FB instances,
    /// program-level declarations.
    ///
    /// Powered by the same extraction the editor's hover and
    /// completion providers use, so what an agent sees here matches
    /// what shows up under the cursor in the GUI. Use it to confirm
    /// "is there a variable named `temp_setpoint`?" without opening
    /// the editor, or to filter by name when looking for a specific
    /// FB instance.
    ///
    /// Output is the same `VariableInfo` shape `POST /api/symbols`
    /// returns: `{ name, type_name, direction }`. Direction is
    /// `input` / `output` / `internal` / `fb_instance` / `local`.
    #[command(verbatim_doc_comment)]
    Symbols {
        /// POU file (`.st`, `.ld.json`, `.fbd.json`, `.sfc.json`).
        file: PathBuf,
        /// Filter to symbols whose name contains this substring.
        #[arg(long)]
        name: Option<String>,
        /// JSON output on stdout.
        #[arg(long)]
        json: bool,
    },
}

/// Subcommands under `cs runtime`. Each one POSTs / GETs a single
/// HTTP endpoint on the running server; defaults to
/// `http://127.0.0.1:3001` and accepts `--server URL` to point elsewhere
/// (e.g. an edge box reachable via SSH-forwarded port).
#[derive(Subcommand, Debug)]
pub(crate) enum RuntimeCmd {
    /// Halt the scan loop. IO is frozen and `run_round` is skipped
    /// until `resume` or `step`. Variable writes / forces still apply.
    Pause {
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Resume continuous scanning.
    Resume {
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Run N scan cycles then auto-pause.
    Step {
        /// Number of cycles to advance (default 1).
        #[arg(default_value_t = 1)]
        cycles: u32,
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Print the current mode (running / paused / step{N}) and the
    /// list of currently-forced variables.
    Status {
        #[arg(long)]
        json: bool,
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Pin a variable to a value — applied every scan until unforced.
    /// Use a one-shot `cs runtime write` if you want the program to
    /// be able to overwrite it next cycle.
    ///
    /// `value` is a human-readable string; the CLI fetches the
    /// variable's type from the live snapshot and bit-packs it
    /// appropriately:
    ///   * BOOL : "TRUE" / "FALSE" / "1" / "0"
    ///   * INT  : decimal integer (32-bit signed)
    ///   * REAL : decimal float — IEEE-754 bit pattern is sent
    ///
    /// Falls back to int-then-float guessing when the runtime hasn't
    /// reported a snapshot yet.
    #[command(verbatim_doc_comment)]
    Force {
        name: String,
        value: String,
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Release a forced variable.
    Unforce {
        name: String,
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// One-shot write (overwritable by the program next cycle). For a
    /// persistent override use `force`. Same value-encoding rules as
    /// `force`.
    Write {
        name: String,
        value: String,
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[command(flatten)]
        server: ServerOpt,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ProjectCmd {
    /// Validate every POU + the synthesised CONFIGURATION block.
    ///
    /// This is the strongest "is this project shippable?" check —
    /// equivalent to what the IDE's `validate_project` endpoint does.
    /// Exit code is 1 on any error. Use this in CI before a deploy.
    Check {
        /// Project root (containing `project.toml`). Defaults to `.`.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// JSON output on stdout instead of human pretty-print.
        #[arg(long)]
        json: bool,
    },

    /// List POUs, devices, and edges in the project.
    ///
    /// Cheap orientation call. Use this before editing an unfamiliar
    /// project to learn what's there.
    Info {
        /// Project root (containing `project.toml`). Defaults to `.`.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// JSON output on stdout instead of human pretty-print.
        #[arg(long)]
        json: bool,
    },

    /// Create a new project under `~/Documents/IA2/<name>/`.
    Create {
        name: String,
        #[command(flatten)]
        server: ServerOpt,
    },

    /// Open an existing project by absolute path; becomes the active
    /// project on the server until `close` (or another `open`) replaces it.
    Open {
        path: PathBuf,
        #[command(flatten)]
        server: ServerOpt,
    },

    /// Close the currently open project. The runtime is stopped and
    /// state caches are cleared.
    Close {
        #[command(flatten)]
        server: ServerOpt,
    },

    /// List every project the server currently has open, plus which
    /// one is the active fallback for `--project`-less requests.
    /// Useful when scripting against a multi-window IDE: pick a name
    /// from this list, then pass it to subsequent commands via the
    /// top-level `--project` flag.
    List {
        /// JSON output on stdout instead of human pretty-print.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        server: ServerOpt,
    },
}

// =================================================================
//   cs device — CRUD on devices
// =================================================================
#[derive(Subcommand, Debug)]
pub(crate) enum DeviceCmd {
    /// Create an empty device of the given protocol. Channels (the
    /// per-coil / per-PDO addresses) default to empty — populate
    /// them via `cs device set --from cfg.json`.
    Create {
        /// Device name (project-unique, used as the iomap key).
        name: String,
        #[arg(long, value_parser = ["modbus","ethercat"])]
        protocol: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// List every device in the open project (name + protocol).
    List {
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Dump the full device config (protocol-specific) as JSON. Use
    /// before `set --from` to edit a snapshot rather than build the
    /// shape from scratch.
    Get {
        name: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Replace a device's entire config from a JSON file. The shape
    /// is the same one `get` returns — round-trip-friendly.
    Set {
        name: String,
        /// Path to a JSON file matching the `Device` shape. Pass `-`
        /// to read from stdin.
        #[arg(long)]
        from: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Delete a device. Any iomap bindings against it are left in
    /// place but will warn-skip at run time.
    Delete {
        name: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Assemble a modular EtherCAT coupler's channels from its ESI file
    /// (the device's `bringup.esi_path`) + the modules it reports. The
    /// device must be EtherCAT with `bringup = esi_modular`. The detected
    /// module idents you pass (slot order) REPLACE the device's channel
    /// list — for a modular coupler the ESI is authoritative.
    EsiAssemble {
        name: String,
        /// Comma-separated module idents in slot order — hex (`0x10`) or
        /// decimal (`16`). Read these off the coupler's `0xF050` scan, or
        /// the modules you've physically installed.
        #[arg(long)]
        idents: String,
        #[command(flatten)]
        server: ServerOpt,
    },
}

// =================================================================
//   cs edge — CRUD on deploy targets
// =================================================================
#[derive(Subcommand, Debug)]
pub(crate) enum EdgeCmd {
    /// Create an edge entry. `host` is anything ssh(1) accepts —
    /// `user@host`, a `~/.ssh/config` alias, etc.
    Create {
        name: String,
        #[arg(long)]
        host: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// List every edge in the open project.
    List {
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Dump the full edge config as JSON (host, ssh_port, ssh_user,
    /// install_dir, runtime_port, notes).
    Get {
        name: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Replace an edge's full config from a JSON file. Shape matches
    /// `get` output. Use this to set `install_dir` or `runtime_port`.
    Set {
        name: String,
        #[arg(long)]
        from: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Delete an edge. If a tunnel is attached for it, it's torn
    /// down at the same time.
    Delete {
        name: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Tail recent log lines from the edge runtime (over ssh). Surfaces
    /// EtherCAT discovery, bus health, and device connect errors that
    /// `probe` (health only) can't show.
    Logs {
        /// Edge name (entry in the open project's edge list).
        name: String,
        /// How many recent lines to fetch (default 200, capped 2000).
        #[arg(long, default_value_t = 200)]
        tail: usize,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Scan the edge bus: per-device connect status + discovered EtherCAT
    /// topology (slave index/name/vendor/product + PDI byte sizes). Author
    /// PDO maps against this real-bus view.
    Scan {
        /// Edge name (entry in the open project's edge list).
        name: String,
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// List the edge's interfaces, serial ports, and arch — pick a NIC
    /// for an EtherCAT device or a /dev/tty* for a Modbus RTU device.
    System {
        /// Edge name (entry in the open project's edge list).
        name: String,
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        server: ServerOpt,
    },
}

// =================================================================
//   cs hmi — operator screens: generate a baseline, then edit it
//   element by element (each op renders live in the IDE canvas)
// =================================================================
#[derive(Subcommand, Debug)]
pub(crate) enum HmiCmd {
    /// List the project's screens (path, title, ISA level).
    List {
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Print one screen's full JSON document.
    Get {
        /// Screen slug (slash-separated, no extension), e.g. `overview`.
        path: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Create an empty screen (then build it up with `cs hmi op`).
    Create {
        path: String,
        /// Operator-facing title (defaults to the slug).
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Replace a whole screen from a JSON file / stdin. Prefer `op` for
    /// incremental edits — `save` refreshes canvases without the
    /// per-element animation.
    Save {
        path: String,
        /// Read the HmiDoc JSON from this file (or `-` for stdin).
        #[arg(long)]
        from: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Apply structured edits — the incremental authoring surface.
    ///
    /// Body is `{"ops":[...]}` or a bare `[...]` of ops. Each op is one
    /// of:
    ///   {"op":"add_node","parent":null,"node":{...}}   append an element
    ///   {"op":"update_node","id":"n1","patch":{...}}   shallow-merge
    ///   {"op":"remove_node","id":"n1"}
    ///   {"op":"set_meta","title":"...","level":2}
    /// Batches apply atomically; the response lists `touched` node ids
    /// (also broadcast over SSE so open canvases animate exactly those
    /// elements). Generate the node shapes from `cs hmi symbols` + the
    /// skill's HMI reference.
    Op {
        path: String,
        /// Read the ops JSON from this file (or `-` for stdin).
        #[arg(long)]
        from: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Structural validation + variable-existence warnings.
    Check {
        path: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Deterministic first-pass screen from project truth (alarmbar,
    /// per-POU sections, indicators/values/setpoints, one trend). 409 if
    /// the screen exists — pass --force to regenerate.
    Generate {
        path: String,
        #[arg(long)]
        force: bool,
        /// Operator-facing title for the generated screen.
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Print the built-in symbol palette: every symbol's bindable keys,
    /// props and default size — the contract `add_node` symbols follow.
    Symbols {
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Delete a screen.
    Delete {
        path: String,
        #[command(flatten)]
        server: ServerOpt,
    },
}

// =================================================================
//   cs iomap — read / write the variable-to-channel binding table
// =================================================================
#[derive(Subcommand, Debug)]
pub(crate) enum IomapCmd {
    /// Print the project's current IoMap as JSON.
    Get {
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Replace the entire IoMap from a JSON file. The shape matches
    /// `get` output: `{ mappings: [{ application, variable, device,
    /// channel, direction }] }`. `application` is the PROGRAM the
    /// variable belongs to — omitting it is the most common cause of
    /// a 422 here.
    Set {
        #[arg(long)]
        from: String,
        #[command(flatten)]
        server: ServerOpt,
    },
}

// =================================================================
//   cs northbound — read / write northbound.toml (MQTT publishing)
// =================================================================
#[derive(Subcommand, Debug)]
pub(crate) enum NorthboundCmd {
    /// Print the project's northbound config as JSON.
    Get {
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Replace northbound.toml from a JSON file. Shape matches `get`:
    /// `{ "mqtt": { "broker_host": …, "publish_interval_ms": …, … } }`.
    Set {
        #[arg(long)]
        from: String,
        #[command(flatten)]
        server: ServerOpt,
    },
}

// =================================================================
//   cs library — list / import / remove FB libraries
// =================================================================
#[derive(Subcommand, Debug)]
pub(crate) enum LibraryCmd {
    /// List registry libraries with their version and per-project
    /// import state. Add `--json` for the raw `LibrarySummary[]`.
    List {
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Vendor library blocks into the open project under
    /// `pous/lib/<name>/`. Omit `--blocks` to import the whole
    /// library; re-importing overwrites (that's the update path).
    Import {
        /// Registry library name, e.g. `process-control`.
        library: String,
        /// Comma-separated block file names (`fb_pid.st,fb_ramp.st`).
        /// Omit to import every block in the library.
        #[arg(long, value_delimiter = ',')]
        blocks: Vec<String>,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Remove an imported library — drops `pous/lib/<name>/` and the
    /// project.toml entry. Idempotent.
    Remove {
        /// Imported library name.
        name: String,
        #[command(flatten)]
        server: ServerOpt,
    },
}

// =================================================================
//   cs tasks — read / write tasks.toml
// =================================================================
#[derive(Subcommand, Debug)]
pub(crate) enum TasksCmd {
    /// Print the project's current Tasks (tasks + program bindings)
    /// as JSON.
    Get {
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Replace the entire tasks.toml content from a JSON file.
    /// Shape: `{ tasks: [{name, interval_ms, priority}], programs:
    /// [{instance, program, task}] }`.
    Set {
        #[arg(long)]
        from: String,
        #[command(flatten)]
        server: ServerOpt,
    },
}

// =================================================================
//   cs agent — explicit takeover-session enter / leave / wrap
// =================================================================
#[derive(Subcommand, Debug)]
pub(crate) enum AgentCmd {
    /// Wrap a command in an agent takeover session. The IDE banner
    /// stays on with `--label` text for the entire duration, instead
    /// of flickering between every `cs` call. On exit (whether the
    /// inner command succeeds, fails, or is interrupted), the
    /// session is closed cleanly.
    ///
    /// Example:
    ///   `cs agent run --label "build tank demo" -- bash -c 'cs ...; cs ...'`
    #[command(verbatim_doc_comment)]
    Run {
        /// Banner text shown in the IDE overlay while the session is
        /// open. Pick something the user will recognise — "rebuilding
        /// tank controller" reads better than "agent".
        #[arg(long)]
        label: String,
        #[command(flatten)]
        server: ServerOpt,
        /// The command + args to run. Use `--` to separate from `cs`
        /// own flags. Example: `cs agent run -l X -- bash -c 'foo'`.
        #[arg(last = true, required = true)]
        cmd: Vec<String>,
    },
    /// Open a session and print its id, then exit. Intended for
    /// shell scripts that want to set `IA2_AGENT_SESSION` and run
    /// many `cs` calls; pair with `cs agent leave` at the end.
    /// Prefer `cs agent run -- cmd` when possible — it cleans up
    /// even on Ctrl-C.
    Enter {
        #[arg(long)]
        label: String,
        #[command(flatten)]
        server: ServerOpt,
    },
    /// Close the agent session whose id is in the
    /// `IA2_AGENT_SESSION` env var (or the value passed to
    /// `--id`). Idempotent — no-op when nothing's open.
    Leave {
        #[arg(long)]
        id: Option<String>,
        #[command(flatten)]
        server: ServerOpt,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum PouCmd {
    /// Create an empty POU file in the open project's `pous/` dir.
    /// The server seeds a minimal compileable skeleton for the chosen
    /// language; agents typically `cs pou save` real content right after.
    Create {
        /// Project-relative slash-path under `pous/`, no extension.
        path: String,
        /// IEC language. Determines the on-disk extension:
        /// `st` → `.st`, `ld` → `.ld.json`, `fbd` → `.fbd.json`, `sfc` → `.sfc.json`.
        #[arg(long, value_parser = ["st","ld","fbd","sfc"])]
        language: String,
        /// IEC POU type. Most agents create PROGRAMs.
        #[arg(long, default_value = "program",
              value_parser = ["program","function_block","function"])]
        r#type: String,
        #[command(flatten)]
        server: ServerOpt,
    },

    /// Overwrite a POU's source. Body is read from `--from <file>`,
    /// `--stdin`, or — if neither is given — from stdin.
    Save {
        /// POU path (same form as `cs pou create`).
        path: String,
        /// Read source from this file. Useful when the agent already
        /// has the content on disk and wants a one-shot push.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Read source from stdin explicitly. Default behaviour if
        /// neither `--from` nor `--stdin` is passed and stdin isn't a TTY.
        #[arg(long, conflicts_with = "from")]
        stdin: bool,
        #[command(flatten)]
        server: ServerOpt,
    },

    /// Delete a POU file. The runtime is NOT stopped — if the POU was
    /// part of the running schedule, behaviour after delete is
    /// undefined until next `cs run`.
    Delete {
        path: String,
        #[command(flatten)]
        server: ServerOpt,
    },
}

fn main() {
    let args = Cli::parse();

    // Stash the optional --project flag in a process-wide OnceLock
    // so every HTTP helper can pick it up and add the
    // X-IA2-Project header. Single-window users who never pass
    // --project see header-less requests, which is the same wire
    // shape we always shipped — back-compat.
    if let Some(p) = args.project.as_deref() {
        PROJECT_OVERRIDE.set(p.to_string()).ok();
    }

    // Heartbeat: if this command is going to mutate server state,
    // announce to the IDE *before* dispatching. The IDE shows a
    // "takeover" overlay while at least one CLI session is active
    // and aged-out after a few seconds of silence. Best-effort — a
    // server timeout / 404 doesn't fail the command, only suppresses
    // the visual cue.
    if let Some((server, label)) = announce_target(&args.command) {
        announce_agent(server, label);
    }

    let result = match args.command {
        Command::Check {
            files,
            json,
            explain,
        } => cmd_check(&files, json, explain),
        Command::Transpile { file, with_map } => cmd_transpile(&file, with_map),
        Command::Project(ProjectCmd::Check { path, json }) => cmd_project_check(&path, json),
        Command::Project(ProjectCmd::Info { path, json }) => cmd_project_info(&path, json),
        Command::Project(ProjectCmd::Create { name, server }) => {
            cmd_project_create(&name, &server.server)
        }
        Command::Project(ProjectCmd::Open { path, server }) => {
            cmd_project_open(&path, &server.server)
        }
        Command::Project(ProjectCmd::Close { server }) => cmd_project_close(&server.server),
        Command::Project(ProjectCmd::List { json, server }) => {
            cmd_project_list(&server.server, json)
        }
        Command::Pou(p) => cmd_pou(p),
        Command::Run {
            program,
            file,
            server,
        } => cmd_run(program.as_deref(), file.as_deref(), &server.server),
        Command::Stop { server } => cmd_stop(&server.server),
        Command::Explain { code } => cmd_explain(&code),
        Command::Symbols { file, name, json } => cmd_symbols(&file, name.as_deref(), json),
        Command::Runtime(r) => cmd_runtime(r),
        Command::Deploy { name, json, server } => cmd_deploy(&name, json, &server.server),
        Command::Probe { name, json, server } => cmd_probe(&name, json, &server.server),
        Command::Device(d) => cmd_device(d),
        Command::Edge(e) => cmd_edge(e),
        Command::Iomap(i) => cmd_iomap(i),
        Command::Hmi(h) => cmd::hmi::cmd_hmi(h),
        Command::Northbound(n) => cmd_northbound(n),
        Command::Library(l) => cmd_library(l),
        Command::Tasks(t) => cmd_tasks(t),
        Command::Agent(a) => cmd_agent(a),
    };
    match result {
        Ok(exit) => std::process::exit(exit),
        Err(e) => {
            // anyhow's chain printing — gives the agent the full
            // context: "I/O error" → "while reading foo.ld.json" → ...
            let _ = writeln!(std::io::stderr(), "error: {e:#}");
            std::process::exit(3);
        }
    }
}
