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
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use ironplc_bridge::CheckDiagnostic;
use project::{PouLanguage, ProjectStore};

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
enum Command {
    /// Validate a POU file (ST or LD). Primary tool for the
    /// edit-validate-fix loop.
    ///
    /// Returns the same diagnostics as `POST /api/check`. Auto-detects
    /// language from the file extension (`.st` → ST, `.ld.json` → LD).
    /// Multiple files: each is checked independently and all diagnostics
    /// are aggregated. Exit code is 1 if any file has errors.
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },

    /// Stop the running runtime. No-op if nothing is running.
    Stop {
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
enum RuntimeCmd {
    /// Halt the scan loop. IO is frozen and `run_round` is skipped
    /// until `resume` or `step`. Variable writes / forces still apply.
    Pause {
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Resume continuous scanning.
    Resume {
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Run N scan cycles then auto-pause.
    Step {
        /// Number of cycles to advance (default 1).
        #[arg(default_value_t = 1)]
        cycles: u32,
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Print the current mode (running / paused / step{N}) and the
    /// list of currently-forced variables.
    Status {
        #[arg(long)]
        json: bool,
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Release a forced variable.
    Unforce {
        name: String,
        /// Target this edge runtime instead of the local server.
        #[arg(long)]
        edge: Option<String>,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
}

#[derive(Subcommand, Debug)]
enum ProjectCmd {
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },

    /// Open an existing project by absolute path; becomes the active
    /// project on the server until `close` (or another `open`) replaces it.
    Open {
        path: PathBuf,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },

    /// Close the currently open project. The runtime is stopped and
    /// state caches are cleared.
    Close {
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
}

// =================================================================
//   cs device — CRUD on devices
// =================================================================
#[derive(Subcommand, Debug)]
enum DeviceCmd {
    /// Create an empty device of the given protocol. Channels (the
    /// per-coil / per-PDO addresses) default to empty — populate
    /// them via `cs device set --from cfg.json`.
    Create {
        /// Device name (project-unique, used as the iomap key).
        name: String,
        #[arg(long, value_parser = ["modbus","ethercat"])]
        protocol: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// List every device in the open project (name + protocol).
    List {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Dump the full device config (protocol-specific) as JSON. Use
    /// before `set --from` to edit a snapshot rather than build the
    /// shape from scratch.
    Get {
        name: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Replace a device's entire config from a JSON file. The shape
    /// is the same one `get` returns — round-trip-friendly.
    Set {
        name: String,
        /// Path to a JSON file matching the `Device` shape. Pass `-`
        /// to read from stdin.
        #[arg(long)]
        from: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Delete a device. Any iomap bindings against it are left in
    /// place but will warn-skip at run time.
    Delete {
        name: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
}

// =================================================================
//   cs edge — CRUD on deploy targets
// =================================================================
#[derive(Subcommand, Debug)]
enum EdgeCmd {
    /// Create an edge entry. `host` is anything ssh(1) accepts —
    /// `user@host`, a `~/.ssh/config` alias, etc.
    Create {
        name: String,
        #[arg(long)]
        host: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// List every edge in the open project.
    List {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Dump the full edge config as JSON (host, ssh_port, ssh_user,
    /// install_dir, runtime_port, notes).
    Get {
        name: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Replace an edge's full config from a JSON file. Shape matches
    /// `get` output. Use this to set `install_dir` or `runtime_port`.
    Set {
        name: String,
        #[arg(long)]
        from: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Delete an edge. If a tunnel is attached for it, it's torn
    /// down at the same time.
    Delete {
        name: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Scan the edge bus: per-device connect status + discovered EtherCAT
    /// topology (slave index/name/vendor/product + PDI byte sizes). Author
    /// PDO maps against this real-bus view.
    Scan {
        /// Edge name (entry in the open project's edge list).
        name: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// List the edge's interfaces, serial ports, and arch — pick a NIC
    /// for an EtherCAT device or a /dev/tty* for a Modbus RTU device.
    System {
        /// Edge name (entry in the open project's edge list).
        name: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
}

// =================================================================
//   cs iomap — read / write the variable-to-channel binding table
// =================================================================
#[derive(Subcommand, Debug)]
enum IomapCmd {
    /// Print the project's current IoMap as JSON.
    Get {
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Replace the entire IoMap from a JSON file. The shape matches
    /// `get` output: `{ mappings: [{ variable, device, channel,
    /// direction }] }`.
    Set {
        #[arg(long)]
        from: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
}

// =================================================================
//   cs northbound — read / write northbound.toml (MQTT publishing)
// =================================================================
#[derive(Subcommand, Debug)]
enum NorthboundCmd {
    /// Print the project's northbound config as JSON.
    Get {
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Replace northbound.toml from a JSON file. Shape matches `get`:
    /// `{ "mqtt": { "broker_host": …, "publish_interval_ms": …, … } }`.
    Set {
        #[arg(long)]
        from: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
}

// =================================================================
//   cs library — list / import / remove FB libraries
// =================================================================
#[derive(Subcommand, Debug)]
enum LibraryCmd {
    /// List registry libraries with their version and per-project
    /// import state. Add `--json` for the raw `LibrarySummary[]`.
    List {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Remove an imported library — drops `pous/lib/<name>/` and the
    /// project.toml entry. Idempotent.
    Remove {
        /// Imported library name.
        name: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
}

// =================================================================
//   cs tasks — read / write tasks.toml
// =================================================================
#[derive(Subcommand, Debug)]
enum TasksCmd {
    /// Print the project's current Tasks (tasks + program bindings)
    /// as JSON.
    Get {
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Replace the entire tasks.toml content from a JSON file.
    /// Shape: `{ tasks: [{name, interval_ms, priority}], programs:
    /// [{instance, program, task}] }`.
    Set {
        #[arg(long)]
        from: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
}

// =================================================================
//   cs agent — explicit takeover-session enter / leave / wrap
// =================================================================
#[derive(Subcommand, Debug)]
enum AgentCmd {
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
    /// Close the agent session whose id is in the
    /// `IA2_AGENT_SESSION` env var (or the value passed to
    /// `--id`). Idempotent — no-op when nothing's open.
    Leave {
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },
}

#[derive(Subcommand, Debug)]
enum PouCmd {
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
    },

    /// Delete a POU file. The runtime is NOT stopped — if the POU was
    /// part of the running schedule, behaviour after delete is
    /// undefined until next `cs run`.
    Delete {
        path: String,
        #[arg(long, default_value = "http://127.0.0.1:3001")]
        server: String,
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
        Command::Project(ProjectCmd::Create { name, server }) => cmd_project_create(&name, &server),
        Command::Project(ProjectCmd::Open { path, server }) => cmd_project_open(&path, &server),
        Command::Project(ProjectCmd::Close { server }) => cmd_project_close(&server),
        Command::Project(ProjectCmd::List { json, server }) => cmd_project_list(&server, json),
        Command::Pou(p) => cmd_pou(p),
        Command::Run {
            program,
            file,
            server,
        } => cmd_run(program.as_deref(), file.as_deref(), &server),
        Command::Stop { server } => cmd_stop(&server),
        Command::Explain { code } => cmd_explain(&code),
        Command::Symbols { file, name, json } => cmd_symbols(&file, name.as_deref(), json),
        Command::Runtime(r) => cmd_runtime(r),
        Command::Deploy { name, json, server } => cmd_deploy(&name, json, &server),
        Command::Probe { name, json, server } => cmd_probe(&name, json, &server),
        Command::Device(d) => cmd_device(d),
        Command::Edge(e) => cmd_edge(e),
        Command::Iomap(i) => cmd_iomap(i),
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

// =================================================================
//   Subcommand: check
// =================================================================

fn cmd_check(files: &[PathBuf], json: bool, explain: bool) -> Result<i32> {
    let mut all: Vec<FileDiagnostics> = Vec::with_capacity(files.len());
    let mut any_errors = false;

    for file in files {
        let language = language_for_path(file)?;
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let diags = ironplc_bridge::check_pou_source(&source, language);
        if !diags.is_empty() {
            any_errors = true;
        }
        all.push(FileDiagnostics {
            file: file.clone(),
            diagnostics: diags,
        });
    }

    if json {
        let value: serde_json::Value = serde_json::json!({
            "ok": !any_errors,
            "files": all.iter().map(|f| serde_json::json!({
                "file": f.file.to_string_lossy(),
                "diagnostics": &f.diagnostics,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        for f in &all {
            print_diagnostics_human(&f.file, &f.diagnostics, explain);
        }
        let total: usize = all.iter().map(|f| f.diagnostics.len()).sum();
        if total == 0 {
            eprintln!(
                "✓ {} file{} clean",
                files.len(),
                if files.len() == 1 { "" } else { "s" }
            );
        } else {
            eprintln!(
                "✗ {} error{} across {} file{}",
                total,
                if total == 1 { "" } else { "s" },
                all.iter().filter(|f| !f.diagnostics.is_empty()).count(),
                if files.len() == 1 { "" } else { "s" },
            );
        }
    }

    Ok(if any_errors { 1 } else { 0 })
}

struct FileDiagnostics {
    file: PathBuf,
    diagnostics: Vec<CheckDiagnostic>,
}

fn print_diagnostics_human(file: &Path, diags: &[CheckDiagnostic], explain: bool) {
    if diags.is_empty() {
        return;
    }
    let f = file.display();
    for d in diags {
        // Exactly one of ld / fbd / sfc location is populated for
        // graphical POUs; all are None for ST. Order doesn't matter —
        // they're mutually exclusive by construction.
        let loc_hint = if let Some(loc) = &d.ld_location {
            format!(" [{}]", describe_ld_location(loc))
        } else if let Some(loc) = &d.fbd_location {
            format!(" [{}]", describe_fbd_location(loc))
        } else if let Some(loc) = &d.sfc_location {
            format!(" [{}]", describe_sfc_location(loc))
        } else {
            String::new()
        };
        eprintln!(
            "{f}:{}:{}: {} {}{loc_hint}: {}",
            d.start_line, d.start_column, d.severity, d.code, d.message,
        );
        // Context lines under the primary message, indented. These
        // are ironplc's `described` entries — almost always one short
        // structured fragment like `variable=foo` or `type=BOOL`.
        for c in &d.context {
            eprintln!("    {c}");
        }
        // Related labels — point at secondary locations like "did you
        // mean: bar?" or "first declared here". We print them as
        // file:line:col-prefixed notes so they're parseable by the
        // same regex an editor would use to jump.
        for r in &d.related {
            eprintln!(
                "    note: {f}:{}:{}: {}",
                r.start_line, r.start_column, r.message,
            );
        }
        // Full explanation when `--explain` is set. Indent every
        // line by two spaces so the prose is visually nested under
        // the diagnostic rather than competing with it.
        if explain {
            if let Some(expl) = &d.explanation {
                eprintln!();
                for line in expl.lines() {
                    eprintln!("  {line}");
                }
                eprintln!();
            }
        }
    }
}

fn describe_ld_location(loc: &ironplc_bridge::LdLocation) -> String {
    use ironplc_bridge::LdLocation::*;
    match loc {
        Variable { name } => format!("var {name}"),
        Rung { rung_id } => format!("rung {rung_id}"),
        Coil {
            rung_id,
            coil_index,
        } => format!("rung {rung_id} · coil {coil_index}"),
        FbCall { rung_id, instance } => format!("rung {rung_id} · {instance}(…)"),
    }
}

fn describe_fbd_location(loc: &ironplc_bridge::FbdLocation) -> String {
    use ironplc_bridge::FbdLocation::*;
    match loc {
        Variable { name } => format!("var {name}"),
        Block { block_id } => format!("block {block_id}"),
        Output { variable } => format!("output {variable}"),
    }
}

fn describe_sfc_location(loc: &ironplc_bridge::SfcLocation) -> String {
    use ironplc_bridge::SfcLocation::*;
    match loc {
        Variable { name } => format!("var {name}"),
        Step { name } => format!("step {name}"),
        Action { step, action_index } => format!("step {step} · action {action_index}"),
        Transition { index } => format!("transition #{index}"),
    }
}

// =================================================================
//   Subcommand: transpile
// =================================================================

fn cmd_transpile(file: &Path, with_map: bool) -> Result<i32> {
    let language = language_for_path(file)?;
    let source =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;

    match language {
        PouLanguage::St => {
            // ST is its own intermediate; nothing to do. Echo it so the
            // command remains useful in pipelines that don't care about
            // language at the caller's side.
            if with_map {
                eprintln!("note: --with-map has no effect for ST sources");
            }
            print!("{source}");
            Ok(0)
        }
        PouLanguage::Ld => {
            let prog: project::LdProgram = serde_json::from_str(&source)
                .with_context(|| format!("parsing LD JSON in {}", file.display()))?;
            let (st, map) = ironplc_bridge::transpile_ld_to_st_with_map(&prog)
                .with_context(|| format!("transpiling {}", file.display()))?;
            if with_map {
                // Serialise the map alongside the ST — JSON output, one
                // pair per call. The map.lines field is a Vec<Option<…>>
                // which serde renders as `[null, {…}, null, …]`.
                let payload = serde_json::json!({
                    "st": st,
                    "source_map": map.lines,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{st}");
            }
            Ok(0)
        }
        PouLanguage::Fbd => {
            let prog: project::FbdProgram = serde_json::from_str(&source)
                .with_context(|| format!("parsing FBD JSON in {}", file.display()))?;
            let (st, map) = ironplc_bridge::transpile_fbd_to_st_with_map(&prog)
                .with_context(|| format!("transpiling {}", file.display()))?;
            if with_map {
                let payload = serde_json::json!({
                    "st": st,
                    "source_map": map.lines,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{st}");
            }
            Ok(0)
        }
        PouLanguage::Sfc => {
            let prog: project::SfcProgram = serde_json::from_str(&source)
                .with_context(|| format!("parsing SFC JSON in {}", file.display()))?;
            let (st, map) = ironplc_bridge::transpile_sfc_to_st_with_map(&prog)
                .with_context(|| format!("transpiling {}", file.display()))?;
            if with_map {
                let payload = serde_json::json!({
                    "st": st,
                    "source_map": map.lines,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{st}");
            }
            Ok(0)
        }
        other => bail!("transpile: language {other:?} is not yet supported"),
    }
}

// =================================================================
//   Subcommand: project check
// =================================================================

fn cmd_project_check(path: &Path, json: bool) -> Result<i32> {
    let store = open_project(path)?;
    let outcome = ironplc_bridge::compile_project(&store);
    let (ok, message): (bool, String) = match outcome {
        Ok(_) => (true, "clean".into()),
        Err(e) => (false, format!("{e:?}")),
    };

    if json {
        let value = serde_json::json!({
            "ok": ok,
            "project": store.name(),
            "message": message,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if ok {
        eprintln!("✓ project {} compiles cleanly", store.name());
    } else {
        eprintln!("✗ project {} failed to compile:", store.name());
        eprintln!("{message}");
    }

    Ok(if ok { 0 } else { 1 })
}

// =================================================================
//   Subcommand: project info
// =================================================================

fn cmd_project_info(path: &Path, json: bool) -> Result<i32> {
    let store = open_project(path)?;
    let pous = store
        .list_pou_paths()
        .with_context(|| "listing POU files")?;
    let devices = store.list_devices().with_context(|| "listing devices")?;
    let edges = store.list_edges().with_context(|| "listing edges")?;

    if json {
        let value = serde_json::json!({
            "name": store.name(),
            "root": store.root().display().to_string(),
            "pous": pous,
            "devices": devices.iter().map(|d| serde_json::json!({
                "name": &d.name,
                "protocol": format!("{:?}", d.config.protocol()),
            })).collect::<Vec<_>>(),
            "edges": edges.iter().map(|e| serde_json::json!({
                "name": &e.name,
                "host": &e.host,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("project: {}", store.name());
        println!("root:    {}", store.root().display());
        println!();
        println!("POUs ({}):", pous.len());
        for p in &pous {
            println!("  {p}");
        }
        println!();
        println!("Devices ({}):", devices.len());
        for d in &devices {
            println!("  {} ({:?})", d.name, d.config.protocol());
        }
        println!();
        println!("Edges ({}):", edges.len());
        for e in &edges {
            println!("  {} → {}", e.name, e.host);
        }
    }

    Ok(0)
}

// =================================================================
//   Subcommand: explain
// =================================================================

fn cmd_explain(code: &str) -> Result<i32> {
    match ironplc_bridge::lookup_problem_doc(code) {
        Some((rst, title)) => {
            // Print the title line first so a quick `cs explain P4007`
            // tells you what the code is for without scanning the body.
            // The full RST follows verbatim — agents and humans can
            // both read it. (rST format is text-friendly so we don't
            // try to render it.)
            println!("{code} — {title}");
            println!();
            print!("{rst}");
            Ok(0)
        }
        None => {
            eprintln!("error: no documentation for `{code}` — not in ironplc's problem registry");
            Ok(1)
        }
    }
}

// =================================================================
//   Subcommand: project create / open / close
// =================================================================
//
// Wrap the HTTP API so agents call `cs project create foo` instead
// of `curl -X POST localhost:3001/api/projects -d '{"name":"foo"}'`.
// Symmetric with `cs project info / check` which already operate on
// project directories.

fn cmd_project_create(name: &str, server: &str) -> Result<i32> {
    let resp = post_json(
        &format!("{server}/api/projects"),
        &serde_json::json!({ "name": name }),
    )?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(0)
}

fn cmd_project_open(path: &Path, server: &str) -> Result<i32> {
    let abs = path
        .canonicalize()
        .with_context(|| format!("resolving {}", path.display()))?;
    let resp = post_json(
        &format!("{server}/api/projects/open"),
        &serde_json::json!({ "path": abs.display().to_string() }),
    )?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(0)
}

fn cmd_project_close(server: &str) -> Result<i32> {
    let resp = post_json(&format!("{server}/api/projects/close"), &())?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(0)
}

fn cmd_project_list(server: &str, json: bool) -> Result<i32> {
    let value = get_json(&format!("{server}/api/projects/open-list"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(0);
    }
    // Human-readable: active marked with `*`, names padded into a
    // column. Path on the right for orientation.
    let active = value
        .get("active")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let projects = value
        .get("projects")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if projects.is_empty() {
        eprintln!("no projects open");
        return Ok(0);
    }
    let name_width = projects
        .iter()
        .filter_map(|p| p.get("name").and_then(|v| v.as_str()).map(str::len))
        .max()
        .unwrap_or(0);
    for p in &projects {
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let path = p.get("path").and_then(|v| v.as_str()).unwrap_or("?");
        let marker = if name == active { "*" } else { " " };
        println!("{marker} {name:<name_width$}  {path}");
    }
    eprintln!(
        "{} project{} open · active marked with *",
        projects.len(),
        if projects.len() == 1 { "" } else { "s" },
    );
    Ok(0)
}

// =================================================================
//   Subcommand: pou create / save / delete
// =================================================================

fn cmd_pou(cmd: PouCmd) -> Result<i32> {
    match cmd {
        PouCmd::Create {
            path,
            language,
            r#type,
            server,
        } => {
            let resp = post_json(
                &format!("{server}/api/pous"),
                // Server's CreatePouRequest uses `type` (renamed
                // from Rust `type_` via serde). Language values match
                // the on-disk extensions: st / ld / fbd / sfc.
                &serde_json::json!({
                    "path": path,
                    "type": r#type,
                    "language": language,
                }),
            )?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        PouCmd::Save {
            path,
            from,
            stdin,
            server,
        } => {
            let source = if let Some(file) = from {
                std::fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))?
            } else {
                // Read stdin (whether `--stdin` is set or it's the
                // implicit default).
                let _ = stdin;
                let mut s = String::new();
                use std::io::Read;
                std::io::stdin()
                    .read_to_string(&mut s)
                    .context("reading source from stdin")?;
                s
            };
            // `save_pou` accepts text/plain, not JSON — wire format
            // matches the IDE editor's auto-save path.
            let url = format!("{server}/api/pous/{}", url_encode(&path));
            let resp = http_agent()
                .put(&url)
                .set("Content-Type", "text/plain")
                .send_string(&source)
                .map_err(|e| anyhow::anyhow!("PUT {url}: {e}"))?;
            let value: serde_json::Value = resp
                .into_json()
                .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(0)
        }
        PouCmd::Delete { path, server } => {
            let resp = delete_json(&format!("{server}/api/pous/{}", url_encode(&path)))?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
    }
}

// =================================================================
//   Subcommand: run / stop
// =================================================================

fn cmd_run(program: Option<&str>, file: Option<&Path>, server: &str) -> Result<i32> {
    // The server distinguishes three run shapes by the presence of
    // `program` / `file_path`. Mirror that here.
    let body = match (program, file) {
        (None, None) => serde_json::json!({ "kind": "project" }),
        (Some(name), None) => serde_json::json!({
            "kind": "isolated",
            "program": name,
        }),
        (Some(name), Some(path)) => {
            let abs = path
                .canonicalize()
                .with_context(|| format!("resolving {}", path.display()))?;
            serde_json::json!({
                "kind": "isolated",
                "program": name,
                "file_path": abs.display().to_string(),
            })
        }
        (None, Some(_)) => {
            anyhow::bail!("--file requires --program to name the PROGRAM inside it")
        }
    };
    let resp = post_json(&format!("{server}/api/run"), &body)?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(0)
}

fn cmd_stop(server: &str) -> Result<i32> {
    let resp = post_json(&format!("{server}/api/stop"), &())?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(0)
}

// =================================================================
//   Subcommand: deploy / probe (edge orchestration)
// =================================================================

fn cmd_deploy(name: &str, json: bool, server: &str) -> Result<i32> {
    // The server's /api/edges/{name}/deploy route owns the SSH+tar
    // dance — see crates/server/src/edges.rs. We just trigger it and
    // surface the report. Bigger timeout than the default agent
    // (30s) because the tar+ssh round-trip can take minutes for a
    // large project on a slow link.
    let url = format!("{server}/api/edges/{}/deploy", url_encode(name));
    let resp = http_agent()
        .post(&url)
        .timeout(std::time::Duration::from_secs(600))
        .set("Content-Type", "application/json")
        .send_json(serde_json::json!({}))
        .map_err(|e| anyhow::anyhow!("POST {url}: {e}"))?;
    let value: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        // Human-readable form: pull the version + the streamed deploy
        // log so the user sees what actually happened on the box.
        let version = value.get("version").and_then(|v| v.as_str()).unwrap_or("?");
        let log = value.get("log").and_then(|v| v.as_str()).unwrap_or("");
        if !log.is_empty() {
            eprintln!("{log}");
        }
        eprintln!("✓ deployed to '{name}' as version {version}");
    }
    // ok=false means the script ran but exited non-zero (remote failure).
    let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    Ok(if ok { 0 } else { 1 })
}

fn cmd_probe(name: &str, json: bool, server: &str) -> Result<i32> {
    let url = format!("{server}/api/edges/{}/probe", url_encode(name));
    let value = get_json(&url)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        let reachable = value
            .get("reachable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if reachable {
            let scans = value
                .get("scan_count")
                .and_then(|v| v.as_u64())
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".into());
            let uptime = value
                .get("uptime_secs")
                .and_then(|v| v.as_u64())
                .map(|n| format!("{n}s"))
                .unwrap_or_else(|| "?".into());
            let version = value
                .get("runtime_version")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("✓ {name} reachable · v{version} · {scans} scans · up {uptime}");
        } else {
            let err = value
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unreachable");
            eprintln!("✗ {name}: {err}");
        }
    }
    let reachable = value
        .get("reachable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Ok(if reachable { 0 } else { 1 })
}

// =================================================================
//   Subcommand: device CRUD
// =================================================================

fn cmd_device(cmd: DeviceCmd) -> Result<i32> {
    match cmd {
        DeviceCmd::Create {
            name,
            protocol,
            server,
        } => {
            let resp = post_json(
                &format!("{server}/api/devices"),
                &serde_json::json!({ "name": name, "protocol": protocol }),
            )?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        DeviceCmd::List { json, server } => {
            // Devices live inside ProjectTree — call /api/project and
            // pluck the `devices` array. Cheap enough; avoids a new
            // dedicated endpoint for what's already exposed.
            let tree = get_json(&format!("{server}/api/project"))?;
            let devices = tree
                .get("devices")
                .cloned()
                .unwrap_or(serde_json::json!([]));
            if json {
                println!("{}", serde_json::to_string_pretty(&devices)?);
            } else if let Some(arr) = devices.as_array() {
                if arr.is_empty() {
                    eprintln!("no devices");
                } else {
                    for d in arr {
                        let n = d.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let p = d.get("protocol").and_then(|v| v.as_str()).unwrap_or("?");
                        println!("{p:<10}  {n}");
                    }
                }
            }
            Ok(0)
        }
        DeviceCmd::Get { name, server } => {
            let resp = get_json(&format!("{server}/api/devices/{}", url_encode(&name)))?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        DeviceCmd::Set { name, from, server } => {
            let body = read_json_blob(&from)?;
            let resp = put_json(
                &format!("{server}/api/devices/{}", url_encode(&name)),
                &body,
            )?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        DeviceCmd::Delete { name, server } => {
            let resp = delete_json(&format!("{server}/api/devices/{}", url_encode(&name)))?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        DeviceCmd::EsiAssemble {
            name,
            idents,
            server,
        } => {
            let detected = idents
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(parse_module_ident)
                .collect::<Result<Vec<u32>>>()?;
            let body = serde_json::json!({ "detected": detected });
            let resp = post_json(
                &format!("{server}/api/devices/{}/esi-assemble", url_encode(&name)),
                &body,
            )?;
            // Summarize the assembled channels. The Device JSON is flat —
            // protocol fields (including `channels`) sit at the top level.
            let n = resp
                .get("channels")
                .and_then(|c| c.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            println!(
                "✓ assembled {n} channels from ESI for '{name}' ({} modules)",
                detected.len()
            );
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
    }
}

/// Parse a module ident in `0x..` hex or decimal form.
fn parse_module_ident(s: &str) -> Result<u32> {
    let t = s.trim();
    let parsed = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(h, 16)
    } else {
        t.parse::<u32>()
    };
    parsed.map_err(|e| anyhow::anyhow!("bad module ident {s:?}: {e}"))
}

// =================================================================
//   Subcommand: edge CRUD
// =================================================================

fn cmd_edge(cmd: EdgeCmd) -> Result<i32> {
    match cmd {
        EdgeCmd::Create { name, host, server } => {
            let resp = post_json(
                &format!("{server}/api/edges"),
                &serde_json::json!({ "name": name, "host": host }),
            )?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        EdgeCmd::List { json, server } => {
            let tree = get_json(&format!("{server}/api/project"))?;
            let edges = tree.get("edges").cloned().unwrap_or(serde_json::json!([]));
            if json {
                println!("{}", serde_json::to_string_pretty(&edges)?);
            } else if let Some(arr) = edges.as_array() {
                if arr.is_empty() {
                    eprintln!("no edges");
                } else {
                    for e in arr {
                        let n = e.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let h = e.get("host").and_then(|v| v.as_str()).unwrap_or("?");
                        println!("{n:<24}  {h}");
                    }
                }
            }
            Ok(0)
        }
        EdgeCmd::Get { name, server } => {
            let resp = get_json(&format!("{server}/api/edges/{}", url_encode(&name)))?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        EdgeCmd::Set { name, from, server } => {
            let body = read_json_blob(&from)?;
            let resp = put_json(&format!("{server}/api/edges/{}", url_encode(&name)), &body)?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        EdgeCmd::Delete { name, server } => {
            let resp = delete_json(&format!("{server}/api/edges/{}", url_encode(&name)))?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        EdgeCmd::Logs { name, tail, server } => {
            let url = format!("{server}/api/edges/{}/logs?tail={tail}", url_encode(&name));
            let resp = get_json(&url)?;
            if let Some(lines) = resp.get("lines").and_then(|v| v.as_array()) {
                for line in lines {
                    if let Some(s) = line.as_str() {
                        println!("{s}");
                    }
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            }
            Ok(0)
        }
        EdgeCmd::Scan { name, json, server } => {
            let url = format!("{server}/api/edges/{}/discover", url_encode(&name));
            let resp = get_json(&url)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
                return Ok(0);
            }
            let Some(devs) = resp.as_array() else {
                println!("{}", serde_json::to_string_pretty(&resp)?);
                return Ok(0);
            };
            if devs.is_empty() {
                eprintln!("no devices in project");
            }
            for d in devs {
                let dname = d.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let proto = d.get("protocol").and_then(|v| v.as_str()).unwrap_or("?");
                let connected = d
                    .get("connected")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !connected {
                    let err = d
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("not connected");
                    println!("✗ {dname} ({proto}) — {err}");
                    continue;
                }
                let slaves = d.get("slaves").and_then(|v| v.as_array());
                let n = slaves.map(|a| a.len()).unwrap_or(0);
                println!("✓ {dname} ({proto}) connected · {n} slave(s)");
                if let Some(arr) = slaves {
                    for s in arr {
                        let idx = s.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                        let sn = s.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let vid = s.get("vendor_id").and_then(|v| v.as_u64()).unwrap_or(0);
                        let pid = s.get("product_id").and_then(|v| v.as_u64()).unwrap_or(0);
                        let inb = s.get("input_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
                        let outb = s.get("output_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
                        println!(
                            "    [{idx}] {sn}  vendor=0x{vid:08x} product=0x{pid:08x}  in={inb}B out={outb}B"
                        );
                    }
                }
            }
            Ok(0)
        }
        EdgeCmd::System { name, json, server } => {
            let url = format!("{server}/api/edges/{}/system", url_encode(&name));
            let resp = get_json(&url)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
                return Ok(0);
            }
            let arch = resp.get("arch").and_then(|v| v.as_str()).unwrap_or("?");
            let os = resp.get("os").and_then(|v| v.as_str()).unwrap_or("?");
            println!("{os}/{arch}");
            if let Some(nics) = resp.get("nics").and_then(|v| v.as_array()) {
                println!("NICs:");
                for n in nics {
                    let nm = n.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let st = n.get("operstate").and_then(|v| v.as_str()).unwrap_or("?");
                    let carrier = n.get("carrier").and_then(|v| v.as_bool()).unwrap_or(false);
                    let mac = n.get("mac").and_then(|v| v.as_str()).unwrap_or("");
                    let link = if carrier { "carrier" } else { "no-carrier" };
                    println!("  {nm:<16} {st:<8} {link:<11} {mac}");
                }
            }
            match resp.get("serial_ports").and_then(|v| v.as_array()) {
                Some(ports) if !ports.is_empty() => {
                    println!("serial ports:");
                    for p in ports {
                        if let Some(s) = p.as_str() {
                            println!("  {s}");
                        }
                    }
                }
                _ => println!("serial ports: (none)"),
            }
            Ok(0)
        }
    }
}

// =================================================================
//   Subcommand: iomap / tasks  (small read/write helpers)
// =================================================================

fn cmd_iomap(cmd: IomapCmd) -> Result<i32> {
    match cmd {
        IomapCmd::Get { server } => {
            let resp = get_json(&format!("{server}/api/iomap"))?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        IomapCmd::Set { from, server } => {
            let body = read_json_blob(&from)?;
            let resp = put_json(&format!("{server}/api/iomap"), &body)?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
    }
}

fn cmd_tasks(cmd: TasksCmd) -> Result<i32> {
    match cmd {
        TasksCmd::Get { server } => {
            let resp = get_json(&format!("{server}/api/tasks"))?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        TasksCmd::Set { from, server } => {
            let body = read_json_blob(&from)?;
            let resp = put_json(&format!("{server}/api/tasks"), &body)?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
    }
}

fn cmd_northbound(cmd: NorthboundCmd) -> Result<i32> {
    match cmd {
        NorthboundCmd::Get { server } => {
            let resp = get_json(&format!("{server}/api/northbound"))?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        NorthboundCmd::Set { from, server } => {
            let body = read_json_blob(&from)?;
            let resp = put_json(&format!("{server}/api/northbound"), &body)?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
    }
}

fn cmd_library(cmd: LibraryCmd) -> Result<i32> {
    match cmd {
        LibraryCmd::List { json, server } => {
            let resp = get_json(&format!("{server}/api/library"))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else if let Some(arr) = resp.as_array() {
                // Concise table: name · version · import state.
                for l in arr {
                    let name = l.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let version = l.get("version").and_then(|v| v.as_str()).unwrap_or("?");
                    let files = l
                        .get("imported_files")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    match l.get("imported_version").and_then(|v| v.as_str()) {
                        Some(iv) => println!(
                            "{name}  v{version}  imported(v{iv}, {files} block{})",
                            if files == 1 { "" } else { "s" }
                        ),
                        None => println!("{name}  v{version}  (not imported)"),
                    }
                }
            }
            Ok(0)
        }
        LibraryCmd::Import {
            library,
            blocks,
            server,
        } => {
            let body = serde_json::json!({ "library": library, "blocks": blocks });
            let resp = post_json(&format!("{server}/api/library/import"), &body)?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        LibraryCmd::Remove { name, server } => {
            let resp = delete_json(&format!("{server}/api/library/{}", url_encode(&name)))?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
    }
}

// =================================================================
//   Subcommand: agent — explicit takeover session
// =================================================================

/// Env var holding the active session id between `cs agent enter`
/// and `cs agent leave`. Set by the user (`export
/// IA2_AGENT_SESSION=$(cs agent enter --label ...)`) or by `cs
/// agent run` when it spawns the inner command.
const SESSION_ENV: &str = "IA2_AGENT_SESSION";

fn cmd_agent(cmd: AgentCmd) -> Result<i32> {
    match cmd {
        AgentCmd::Run { label, server, cmd } => cmd_agent_run(&label, &server, cmd),
        AgentCmd::Enter { label, server } => {
            let id = session_id().to_string();
            agent_session_start(&server, &id, &label)?;
            // Print the id on stdout so shell scripts can capture it:
            //   SESSION=$(cs agent enter --label ...)
            //   ...
            //   cs agent leave --id "$SESSION"
            println!("{id}");
            Ok(0)
        }
        AgentCmd::Leave { id, server } => {
            let target = id.or_else(|| std::env::var(SESSION_ENV).ok());
            let body = match target {
                Some(id) => serde_json::json!({ "id": id }),
                None => serde_json::json!({}),
            };
            let _ = post_json(&format!("{server}/api/agent/session/end"), &body)?;
            Ok(0)
        }
    }
}

fn cmd_agent_run(label: &str, server: &str, cmd: Vec<String>) -> Result<i32> {
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    if cmd.is_empty() {
        anyhow::bail!("cs agent run: expected a command after `--`");
    }

    // Generate session id. We reuse the same per-process id helper
    // the heartbeat path uses so a session id is comparable in logs
    // to a heartbeat session hint.
    let id = session_id().to_string();
    agent_session_start(server, &id, label)?;

    // Background heartbeat keeper. Every second, refresh the
    // session-side last_heartbeat so the server-side watchdog
    // (SESSION_TTL = 30s) doesn't age us out mid-execution.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_keeper = stop.clone();
    let server_owned = server.to_string();
    let id_for_keeper = id.clone();
    let keeper = std::thread::spawn(move || {
        while !stop_for_keeper.load(Ordering::Relaxed) {
            // Best-effort — short timeout, swallow errors. A failed
            // heartbeat only matters after SESSION_TTL of failures
            // in a row.
            let _ = http_agent()
                .post(&format!("{server_owned}/api/agent/heartbeat"))
                .timeout(std::time::Duration::from_millis(500))
                .set("Content-Type", "application/json")
                .send_json(serde_json::json!({
                    "command": null,
                    "session": id_for_keeper,
                }));
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });

    // Run the inner command. Expose the session id in its env so
    // any cs subcalls within `bash -c '...'` carry the same session.
    let mut child = Command::new(&cmd[0]);
    child
        .args(&cmd[1..])
        .env(SESSION_ENV, &id)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = child
        .status()
        .with_context(|| format!("spawning `{}`", cmd[0]))?;

    // Cleanup: stop the keeper, then close the session. Done in
    // try/finally style — even if the inner command crashed, we
    // close so the overlay doesn't get stuck on.
    stop.store(true, Ordering::Relaxed);
    let _ = keeper.join();
    let _ = post_json(
        &format!("{server}/api/agent/session/end"),
        &serde_json::json!({ "id": id }),
    );

    Ok(status.code().unwrap_or(1))
}

/// Open a session on the server. Errors propagate so the caller
/// can decide whether to still run the wrapped command — current
/// policy is "fail fast" since the user explicitly asked for
/// session-mode visual feedback.
fn agent_session_start(server: &str, id: &str, label: &str) -> Result<()> {
    let url = format!("{server}/api/agent/session/start");
    let resp = http_agent()
        .post(&url)
        .set("Content-Type", "application/json")
        .send_json(serde_json::json!({ "id": id, "label": label }))
        .map_err(|e| anyhow::anyhow!("POST {url}: {e}"))?;
    // Drain the body so the connection can be reused.
    let _: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
    Ok(())
}

/// Shared helper: read a JSON document from a file path, or from
/// stdin if `from == "-"`. Used by every `set --from` subcommand
/// so the shape is consistent (matches what `cs pou save` already
/// does for source text).
fn read_json_blob(from: &str) -> Result<serde_json::Value> {
    use std::io::Read;
    let bytes = if from == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        buf.into_bytes()
    } else {
        std::fs::read(from).with_context(|| format!("reading {from}"))?
    };
    serde_json::from_slice(&bytes).with_context(|| format!("parsing JSON from {from}"))
}

fn put_json(url: &str, body: &impl serde::Serialize) -> Result<serde_json::Value> {
    let resp = with_project_header(http_agent().put(url))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| anyhow::anyhow!("PUT {url}: {e}"))?;
    let value: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
    Ok(value)
}

// =================================================================
//   Subcommand: symbols
// =================================================================

fn cmd_symbols(file: &Path, name_filter: Option<&str>, json: bool) -> Result<i32> {
    let language = language_for_path(file)?;
    let source =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let mut syms = ironplc_bridge::extract_symbols(&source, language);
    if let Some(needle) = name_filter {
        syms.retain(|s| s.name.contains(needle));
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&syms)?);
    } else {
        // Tabular: aligned `direction  name : type_name`. Direction
        // pads to the widest width so columns line up.
        let pad = syms.iter().map(|s| s.direction.len()).max().unwrap_or(0);
        for s in &syms {
            println!(
                "{:<pad$}  {} : {}",
                s.direction,
                s.name,
                s.type_name,
                pad = pad,
            );
        }
        eprintln!(
            "{} symbol{}",
            syms.len(),
            if syms.len() == 1 { "" } else { "s" },
        );
    }
    Ok(if syms.is_empty() && name_filter.is_some() {
        1
    } else {
        0
    })
}

// =================================================================
//   Subcommand: runtime (debug control trio)
// =================================================================

fn cmd_runtime(cmd: RuntimeCmd) -> Result<i32> {
    match cmd {
        RuntimeCmd::Pause { edge, server } => {
            let resp = match &edge {
                Some(e) => post_json(
                    &format!("{server}/api/edges/{}/runtime/pause", url_encode(e)),
                    &serde_json::json!({}),
                )?,
                None => post_json(&format!("{server}/api/runtime/pause"), &())?,
            };
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        RuntimeCmd::Resume { edge, server } => {
            let resp = match &edge {
                Some(e) => post_json(
                    &format!("{server}/api/edges/{}/runtime/resume", url_encode(e)),
                    &serde_json::json!({}),
                )?,
                None => post_json(&format!("{server}/api/runtime/resume"), &())?,
            };
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        RuntimeCmd::Step {
            cycles,
            edge,
            server,
        } => {
            let body = serde_json::json!({ "cycles": cycles });
            let resp = match &edge {
                Some(e) => post_json(
                    &format!("{server}/api/edges/{}/runtime/step", url_encode(e)),
                    &body,
                )?,
                None => post_json(&format!("{server}/api/runtime/step"), &body)?,
            };
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        RuntimeCmd::Status { json, edge, server } => {
            // Local: /api/runtime/status. Edge: the runtime's /status via
            // the server proxy (different shape, but carries mode + forces).
            let status = match &edge {
                Some(e) => get_json(&format!("{server}/api/edges/{}/status", url_encode(e)))?,
                None => get_json(&format!("{server}/api/runtime/status"))?,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                // A minimal human summary; full status is one --json
                // away.
                let mode = status
                    .get("mode")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let forces = status
                    .get("forces")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                // Edge /status has no `running` bool — derive from mode.
                let running = status
                    .get("running")
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| {
                        mode.get("kind").and_then(|k| k.as_str()) == Some("running")
                    });
                println!(
                    "running: {running}  mode: {}  forces: {}",
                    serde_json::to_string(&mode)?,
                    forces.len(),
                );
                for f in &forces {
                    if let (Some(n), Some(v)) =
                        (f.get("name").and_then(|v| v.as_str()), f.get("value"))
                    {
                        println!("  {n} := {v}");
                    }
                }
            }
            Ok(0)
        }
        RuntimeCmd::Force {
            name,
            value,
            edge,
            server,
        } => {
            let resp = match &edge {
                Some(e) => {
                    let encoded =
                        pack_value(&name, edge_var_type(&server, e, &name).as_deref(), &value)?;
                    post_json(
                        &format!("{server}/api/edges/{}/runtime/force", url_encode(e)),
                        &serde_json::json!({ "name": name, "value": encoded }),
                    )?
                }
                None => {
                    let encoded = parse_value(&server, &name, &value)?;
                    post_json(
                        &format!("{server}/api/runtime/forces/{}", url_encode(&name)),
                        &serde_json::json!({ "value": encoded }),
                    )?
                }
            };
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        RuntimeCmd::Unforce { name, edge, server } => {
            let resp = match &edge {
                Some(e) => post_json(
                    &format!("{server}/api/edges/{}/runtime/unforce", url_encode(e)),
                    &serde_json::json!({ "name": name }),
                )?,
                None => delete_json(&format!(
                    "{server}/api/runtime/forces/{}",
                    url_encode(&name)
                ))?,
            };
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
        RuntimeCmd::Write {
            name,
            value,
            edge,
            server,
        } => {
            let resp = match &edge {
                Some(e) => {
                    let encoded =
                        pack_value(&name, edge_var_type(&server, e, &name).as_deref(), &value)?;
                    post_json(
                        &format!("{server}/api/edges/{}/runtime/write", url_encode(e)),
                        &serde_json::json!({ "name": name, "value": encoded }),
                    )?
                }
                None => {
                    let encoded = parse_value(&server, &name, &value)?;
                    post_json(
                        &format!("{server}/api/runtime/variables/{}", url_encode(&name)),
                        &serde_json::json!({ "value": encoded }),
                    )?
                }
            };
            println!("{}", serde_json::to_string_pretty(&resp)?);
            Ok(0)
        }
    }
}

/// Convert a human-typed value into the i32 the runtime wire protocol
/// expects, type-aware via the runtime's snapshot.
///
/// Why: the bridge stores all variables — BOOL, INT, REAL, … — in
/// 32-bit slots and the force/write endpoint takes a raw `i32`. For
/// REAL the i32 is the IEEE-754 bit pattern of the float, NOT the
/// integer value. Without type info, `cs runtime force x 50.0` would
/// have to send `1112014848`. This helper does the conversion so
/// humans (and agents) can use natural notation.
///
/// Strategy:
///   1. If the value is obviously BOOL ("true"/"false" case-insensitive)
///      → 0 / 1.
///   2. Otherwise fetch `/api/runtime/snapshot`, look up the variable,
///      encode based on its `type_name` (REAL → bit-pack, INT-family
///      → as-is).
///   3. If the snapshot doesn't include the variable (runtime not
///      running yet, or the variable lives in a POU instance the
///      bridge's snapshot extractor doesn't traverse — a known bridge
///      bug as of 2026-05), fall back to format-based sniffing: a
///      decimal point implies REAL, otherwise INT. Print a stderr
///      note so users know we guessed.
fn parse_value(server: &str, name: &str, raw: &str) -> Result<i32> {
    let var_type = snapshot_var_type(server, name).unwrap_or_default();
    pack_value(name, var_type.as_deref(), raw)
}

/// Resolve an edge variable's type from the edge runtime's `/status`
/// (last snapshot, which carries per-variable `type_name`).
fn edge_var_type(server: &str, edge: &str, name: &str) -> Option<String> {
    let status = get_json(&format!("{server}/api/edges/{}/status", url_encode(edge))).ok()?;
    let vars = status.get("last_snapshot")?.get("vars")?.as_array()?;
    for v in vars {
        if v.get("name").and_then(|n| n.as_str()) == Some(name) {
            return v
                .get("type_name")
                .and_then(|t| t.as_str())
                .map(String::from);
        }
    }
    None
}

/// Bit-pack a human value string into the i32 force/write wire, given the
/// variable's IEC `var_type` (None = unknown → guess from value format).
fn pack_value(name: &str, var_type: Option<&str>, raw: &str) -> Result<i32> {
    // BOOL shortcuts. Case-insensitive because TRUE/FALSE are the IEC
    // canonical form but agents type either.
    match raw.to_ascii_lowercase().as_str() {
        "true" => return Ok(1),
        "false" => return Ok(0),
        _ => {}
    }

    match var_type {
        Some("BOOL") => {
            // We already handled TRUE/FALSE above; accept 0/1 too.
            let n: i32 = raw.parse().with_context(|| {
                format!("value `{raw}` doesn't fit BOOL (expected TRUE/FALSE/1/0)")
            })?;
            Ok(if n != 0 { 1 } else { 0 })
        }
        Some("REAL") => {
            let f: f32 = raw
                .parse()
                .with_context(|| format!("value `{raw}` doesn't parse as REAL (32-bit float)"))?;
            Ok(f.to_bits() as i32)
        }
        Some("LREAL") => {
            anyhow::bail!(
                "LREAL (64-bit float) doesn't fit the 32-bit force wire — \
                 use a REAL variable, or write the low 32 bits manually"
            )
        }
        Some(int_type)
            if matches!(
                int_type,
                "INT" | "DINT" | "SINT" | "UINT" | "UDINT" | "USINT" | "BYTE" | "WORD" | "DWORD"
            ) =>
        {
            let n: i64 = raw.parse().with_context(|| {
                format!("value `{raw}` doesn't parse as integer for {int_type}")
            })?;
            // Wire is i32; for unsigned and larger types we just bit-
            // truncate. Users wanting precise unsigned semantics can
            // pass the i32 reinterpretation directly.
            Ok(n as i32)
        }
        Some(other) => {
            anyhow::bail!("don't know how to encode value `{raw}` for type {other} (yet)")
        }
        None => {
            // No type info — guess from format and warn loudly.
            if raw.contains('.') || raw.contains('e') || raw.contains('E') {
                let f: f32 = raw.parse().with_context(|| {
                    format!("value `{raw}` looks like a float but doesn't parse as f32")
                })?;
                eprintln!(
                    "note: runtime didn't expose `{name}`'s type — guessed REAL from value format"
                );
                Ok(f.to_bits() as i32)
            } else {
                let n: i32 = raw.parse().with_context(|| {
                    format!("value `{raw}` doesn't parse as i32; if you meant REAL, use `{raw}.0`")
                })?;
                eprintln!("note: runtime didn't expose `{name}`'s type — assumed INT family");
                Ok(n)
            }
        }
    }
}

/// Best-effort variable type lookup via `/api/runtime/snapshot`. The
/// snapshot returns one record per live variable with `type_name`. If
/// the runtime isn't running, or the bridge's extractor doesn't
/// include this variable's POU, return Ok(None) and let the caller
/// fall back to format-sniffing.
fn snapshot_var_type(server: &str, name: &str) -> Result<Option<String>> {
    let snap = match get_json(&format!("{server}/api/runtime/snapshot")) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let vars = match snap.get("vars").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Ok(None),
    };
    for v in vars {
        if v.get("name").and_then(|n| n.as_str()) == Some(name) {
            return Ok(v
                .get("type_name")
                .and_then(|t| t.as_str())
                .map(String::from));
        }
    }
    Ok(None)
}

// =================================================================
//   Agent heartbeat
// =================================================================
//
// Short-lived `cs` commands ping POST /api/agent/heartbeat at start.
// The server keeps the "agent active" flag set for ~3 s after the
// last heartbeat; the IDE renders its takeover overlay while it's
// set. Read-only commands (check, info, status, symbols, explain,
// transpile) deliberately skip the heartbeat — querying state isn't
// "operating" and shouldn't trigger the overlay.

/// Return `Some((server, label))` for commands that should announce
/// before dispatching, `None` for read-only commands. The label is
/// what shows up in the IDE banner ("Agent in control · pou create").
fn announce_target(cmd: &Command) -> Option<(&str, &'static str)> {
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
        Command::Project(ProjectCmd::List { server, .. }) => Some((server, "project list")),
        Command::Project(ProjectCmd::Create { server, .. }) => Some((server, "project create")),
        Command::Project(ProjectCmd::Open { server, .. }) => Some((server, "project open")),
        Command::Project(ProjectCmd::Close { server, .. }) => Some((server, "project close")),

        Command::Pou(PouCmd::Create { server, .. }) => Some((server, "pou create")),
        Command::Pou(PouCmd::Save { server, .. }) => Some((server, "pou save")),
        Command::Pou(PouCmd::Delete { server, .. }) => Some((server, "pou delete")),

        Command::Device(DeviceCmd::List { server, .. }) => Some((server, "device list")),
        Command::Device(DeviceCmd::Get { server, .. }) => Some((server, "device get")),
        Command::Device(DeviceCmd::Create { server, .. }) => Some((server, "device create")),
        Command::Device(DeviceCmd::Set { server, .. }) => Some((server, "device set")),
        Command::Device(DeviceCmd::Delete { server, .. }) => Some((server, "device delete")),
        Command::Device(DeviceCmd::EsiAssemble { server, .. }) => {
            Some((server, "device esi-assemble"))
        }

        Command::Edge(EdgeCmd::List { server, .. }) => Some((server, "edge list")),
        Command::Edge(EdgeCmd::Get { server, .. }) => Some((server, "edge get")),
        Command::Edge(EdgeCmd::Logs { server, .. }) => Some((server, "edge logs")),
        Command::Edge(EdgeCmd::Scan { server, .. }) => Some((server, "edge scan")),
        Command::Edge(EdgeCmd::System { server, .. }) => Some((server, "edge system")),
        Command::Edge(EdgeCmd::Create { server, .. }) => Some((server, "edge create")),
        Command::Edge(EdgeCmd::Set { server, .. }) => Some((server, "edge set")),
        Command::Edge(EdgeCmd::Delete { server, .. }) => Some((server, "edge delete")),

        Command::Iomap(IomapCmd::Get { server, .. }) => Some((server, "iomap get")),
        Command::Iomap(IomapCmd::Set { server, .. }) => Some((server, "iomap set")),
        Command::Tasks(TasksCmd::Get { server, .. }) => Some((server, "tasks get")),
        Command::Northbound(NorthboundCmd::Get { server, .. }) => Some((server, "northbound get")),
        Command::Northbound(NorthboundCmd::Set { server, .. }) => Some((server, "northbound set")),
        Command::Tasks(TasksCmd::Set { server, .. }) => Some((server, "tasks set")),

        Command::Library(LibraryCmd::List { server, .. }) => Some((server, "library list")),
        Command::Library(LibraryCmd::Import { server, .. }) => Some((server, "library import")),
        Command::Library(LibraryCmd::Remove { server, .. }) => Some((server, "library remove")),

        Command::Probe { server, .. } => Some((server, "probe")),
        Command::Run { server, .. } => Some((server, "run")),
        Command::Stop { server, .. } => Some((server, "stop")),
        Command::Deploy { server, .. } => Some((server, "deploy")),

        Command::Runtime(RuntimeCmd::Status { server, .. }) => Some((server, "runtime status")),
        Command::Runtime(RuntimeCmd::Pause { server, .. }) => Some((server, "runtime pause")),
        Command::Runtime(RuntimeCmd::Resume { server, .. }) => Some((server, "runtime resume")),
        Command::Runtime(RuntimeCmd::Step { server, .. }) => Some((server, "runtime step")),
        Command::Runtime(RuntimeCmd::Force { server, .. }) => Some((server, "runtime force")),
        Command::Runtime(RuntimeCmd::Unforce { server, .. }) => Some((server, "runtime unforce")),
        Command::Runtime(RuntimeCmd::Write { server, .. }) => Some((server, "runtime write")),
    }
}

/// Per-process session id. Generated lazily so commands that don't
/// announce don't pay the cost. Format: `cs-<pid>-<nanos>` — random
/// enough for "tell agents apart" without pulling the uuid crate.
fn session_id() -> &'static str {
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
fn announce_agent(server: &str, command_label: &str) {
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

/// Tiny URL-component escaper. Variable names should be IEC identifiers
/// (alphanumeric + `_`), but operators sometimes write `instance.pin`
/// in `cs runtime force foo.bar` — the dot is safe but slashes
/// wouldn't be. Cover the common cases without pulling a full
/// percent-encoding crate.
fn url_encode(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '~') {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32)
            }
        })
        .collect()
}

/// Build a no-proxy ureq Agent. ureq 2.x auto-picks up `HTTP_PROXY` /
/// `HTTPS_PROXY` env vars at request time, which routes our localhost
/// API traffic through the user's developer proxy (Clash etc.). Users
/// running a system-wide proxy see "Header field didn't end with \n"
/// because their proxy speaks SOCKS / Trojan, not HTTP. Building an
/// explicit Agent with no proxy fixes it.
///
/// We cache the Agent in a OnceLock so each `cs` invocation pays the
/// build cost once even if it makes several requests.
fn http_agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            // No `.proxy(...)` call — ureq treats absence as "direct
            // connection". Without this Agent, the static `ureq::post(...)`
            // path defaults to reading proxy from env.
            .timeout(std::time::Duration::from_secs(30))
            .build()
    })
}

/// Stores the `--project NAME` value parsed off the command line.
/// When present, every HTTP request adds an `X-IA2-Project` header
/// so the server routes the call to the named project; otherwise
/// the header is omitted and the server uses its active fallback
/// (back-compat with all the existing single-window flows).
pub static PROJECT_OVERRIDE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Wrap a `ureq::Request` so it carries the X-IA2-Project header when
/// the user passed `--project NAME`. The builder pattern means each
/// call site is a one-line `with_project_header(http_agent().post(url))`
/// or similar.
fn with_project_header(req: ureq::Request) -> ureq::Request {
    if let Some(name) = PROJECT_OVERRIDE.get() {
        req.set("X-IA2-Project", name)
    } else {
        req
    }
}

fn post_json(url: &str, body: &impl serde::Serialize) -> Result<serde_json::Value> {
    let resp = with_project_header(http_agent().post(url))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| anyhow::anyhow!("POST {url}: {e}"))?;
    let value: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
    Ok(value)
}

fn get_json(url: &str) -> Result<serde_json::Value> {
    let resp = with_project_header(http_agent().get(url))
        .call()
        .map_err(|e| anyhow::anyhow!("GET {url}: {e}"))?;
    let value: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
    Ok(value)
}

fn delete_json(url: &str) -> Result<serde_json::Value> {
    let resp = with_project_header(http_agent().delete(url))
        .call()
        .map_err(|e| anyhow::anyhow!("DELETE {url}: {e}"))?;
    let value: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
    Ok(value)
}

// =================================================================
//   Shared helpers
// =================================================================

/// Map a file path to its POU language by extension. `.ld.json` is the
/// canonical LD extension (see MEMORY/graphical-languages.md); plain
/// `.st` is ST. Anything else is an error rather than a silent default
/// — agents should know which path they're on.
fn language_for_path(path: &Path) -> Result<PouLanguage> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .with_context(|| format!("invalid filename: {}", path.display()))?;
    // Order: longest known suffix first (`.ld.json` must beat `.st`'s
    // would-be ".json" eyeball check; `.fbd.json` and `.sfc.json` must
    // not collide with a generic `.json`).
    if name.ends_with(".ld.json") {
        Ok(PouLanguage::Ld)
    } else if name.ends_with(".fbd.json") {
        Ok(PouLanguage::Fbd)
    } else if name.ends_with(".sfc.json") {
        Ok(PouLanguage::Sfc)
    } else if name.ends_with(".st") {
        Ok(PouLanguage::St)
    } else {
        bail!(
            "can't infer language from filename {name:?} — expected .st, .ld.json, .fbd.json, or .sfc.json"
        )
    }
}

/// Open a project store at `path`. Resolves `.` to the current working
/// directory so `cs project check` (no args) does the right thing.
fn open_project(path: &Path) -> Result<ProjectStore> {
    let abs = if path.as_os_str() == "." {
        std::env::current_dir().context("resolving current directory")?
    } else {
        path.to_path_buf()
    };
    ProjectStore::open(abs.clone()).with_context(|| format!("opening project at {}", abs.display()))
}
