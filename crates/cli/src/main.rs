//! `cs` — agent-first command-line interface for controlsoftware.
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

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use ironplc_bridge::CheckDiagnostic;
use project::{PouLanguage, ProjectStore};

// =================================================================
//   Top-level command surface
// =================================================================

/// `cs` — controlsoftware CLI. Static analysis, transpile, project
/// inspection. Online runtime operations stay on the HTTP API.
#[derive(Parser, Debug)]
#[command(
    name = "cs",
    version,
    about = "controlsoftware CLI — agent-first static analysis & project tools",
    long_about = "\
controlsoftware CLI — agent-first static analysis & project tools.

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
}

fn main() {
    let args = Cli::parse();
    let result = match args.command {
        Command::Check { files, json, explain } => cmd_check(&files, json, explain),
        Command::Transpile { file, with_map } => cmd_transpile(&file, with_map),
        Command::Project(ProjectCmd::Check { path, json }) => cmd_project_check(&path, json),
        Command::Project(ProjectCmd::Info { path, json }) => cmd_project_info(&path, json),
        Command::Explain { code } => cmd_explain(&code),
        Command::Symbols { file, name, json } => cmd_symbols(&file, name.as_deref(), json),
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
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("reading {}", file.display()))?;
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
            eprintln!("✓ {} file{} clean", files.len(), if files.len() == 1 { "" } else { "s" });
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
        Coil { rung_id, coil_index } => format!("rung {rung_id} · coil {coil_index}"),
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
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("reading {}", file.display()))?;

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
            eprintln!(
                "error: no documentation for `{code}` — not in ironplc's problem registry"
            );
            Ok(1)
        }
    }
}

// =================================================================
//   Subcommand: symbols
// =================================================================

fn cmd_symbols(file: &Path, name_filter: Option<&str>, json: bool) -> Result<i32> {
    let language = language_for_path(file)?;
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("reading {}", file.display()))?;
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
    Ok(if syms.is_empty() && name_filter.is_some() { 1 } else { 0 })
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
