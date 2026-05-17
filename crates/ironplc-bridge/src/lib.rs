//! Wraps vendored ironplc parser + analyzer + codegen + VM into a single
//! `compile(source) -> Container` and `spawn(container) -> ProgramHandle` API
//! intended for downstream consumption by the server crate.

mod errors;
mod fbd_transpile;
mod ld_transpile;
mod problem_docs;
mod retain;
mod runtime;
mod sfc_transpile;

pub use problem_docs::{lookup_problem_doc, lookup_problem_explanation};

pub use fbd_transpile::transpile_to_st as transpile_fbd_to_st;
pub use fbd_transpile::{
    transpile_to_st_with_map as transpile_fbd_to_st_with_map, FbdLocation, FbdSourceMap,
};
pub use ld_transpile::transpile_to_st as transpile_ld_to_st;
pub use ld_transpile::{
    transpile_to_st_with_map as transpile_ld_to_st_with_map, LdLocation, LdSourceMap,
};
pub use sfc_transpile::transpile_to_st as transpile_sfc_to_st;
pub use sfc_transpile::{
    transpile_to_st_with_map as transpile_sfc_to_st_with_map, SfcLocation, SfcSourceMap,
};

pub use errors::BridgeError;
pub use runtime::{
    spawn, spawn_with_interval, spawn_with_options, DeviceSpec, ProgramHandle, RuntimeMode,
    RuntimeWriteError, SpawnOptions, VarSnapshot, VarValue, DEFAULT_SCAN_INTERVAL_MS,
    RETAIN_FLUSH_INTERVAL,
};

use ironplc_container::Container;
use ironplc_dsl::common::{
    DeclarationQualifier, InitialValueAssignmentKind, Library, LibraryElementKind, VarDecl,
    VariableType,
};
use ironplc_dsl::core::FileId;
use ironplc_dsl::diagnostic::{Diagnostic, LineColumn};
use ironplc_parser::options::CompilerOptions;
use serde::Serialize;
use ts_rs::TS;

/// Per-program metadata derived from the source's AST that the runtime
/// needs but the bytecode `Container` itself doesn't preserve.
///
/// Right now this is just the set of `VAR RETAIN`-qualified variable
/// names — `ironplc`'s codegen flattens qualifiers away once it lowers
/// to bytecode (see `vendor/ironplc/compiler/container/src/debug_format.rs`),
/// so we re-derive them from the parsed `Library` for the persistence
/// layer to consume.
#[derive(Debug, Clone, Default)]
pub struct ProgramMetadata {
    /// IEC variable names declared with `RETAIN` (or with `retain="true"`
    /// in PLCopen XML). Lower-cased to match `VarDebugInfo.name` which
    /// is what the runtime looks up against. Stable across compilations
    /// of the same source.
    pub retain_vars: Vec<String>,
}

/// Compile an IEC 61131-3 Structured Text source string into an executable
/// ironplc bytecode `Container`. Uses dialect Ed2 with no vendor extensions.
///
/// Thin convenience wrapper around `compile_with_metadata` that drops the
/// metadata; for code paths that *do* need retain info (the run path),
/// call `compile_with_metadata` directly.
pub fn compile(source: &str) -> Result<Container, BridgeError> {
    compile_with_metadata(source).map(|(c, _)| c)
}

/// Like `compile` but also returns the `ProgramMetadata` extracted from
/// the parsed AST (retain vars and anything else the codegen drops).
pub fn compile_with_metadata(source: &str) -> Result<(Container, ProgramMetadata), BridgeError> {
    let file_id = FileId::default();
    let options = CompilerOptions {
        allow_empty_var_blocks: true,
        ..Default::default()
    };

    let library = ironplc_parser::parse_program(source, &file_id, &options)
        .map_err(|d| BridgeError::Parse(format!("{d:?}")))?;

    // Capture retain vars BEFORE we hand the library to the analyzer,
    // which moves/consumes parts of it. The walk is read-only and cheap.
    let metadata = ProgramMetadata {
        retain_vars: extract_retain_vars(&library),
    };

    let (analyzed, context) = ironplc_analyzer::stages::analyze(&[&library], &options)
        .map_err(|ds| BridgeError::Analyze(format!("{ds:?}")))?;

    if context.has_diagnostics() {
        return Err(BridgeError::Analyze(format!("{:?}", context.diagnostics())));
    }

    let codegen_options = ironplc_codegen::CodegenOptions::default();
    let container = ironplc_codegen::compile(&analyzed, &context, &codegen_options)
        .map_err(|d| BridgeError::Codegen(format!("{d:?}")))?;

    Ok((container, metadata))
}

/// Walk a parsed `Library` and collect every variable whose declaration
/// is qualified `RETAIN`. Covers:
///   - `PROGRAM` `VAR RETAIN` blocks
///   - `FUNCTION_BLOCK` `VAR RETAIN` blocks (FB instance state)
///   - Global `VAR_GLOBAL RETAIN` declarations
///
/// We don't currently descend into nested function-block instances
/// (their retain qualifier is on the FB's own declaration, which we
/// already capture). Variable names are lower-cased so the runtime
/// can match directly against `VarDebugInfo.name` (which ironplc's
/// debug section also stores lower-cased).
fn extract_retain_vars(library: &Library) -> Vec<String> {
    let mut out = Vec::new();
    for element in &library.elements {
        match element {
            LibraryElementKind::ProgramDeclaration(p) => {
                for v in &p.variables {
                    if v.qualifier == DeclarationQualifier::Retain {
                        if let Some(id) = v.identifier.symbolic_id() {
                            out.push(id.lower_case().clone());
                        }
                    }
                }
            }
            LibraryElementKind::FunctionBlockDeclaration(fb) => {
                for v in &fb.variables {
                    if v.qualifier == DeclarationQualifier::Retain {
                        if let Some(id) = v.identifier.symbolic_id() {
                            out.push(id.lower_case().clone());
                        }
                    }
                }
            }
            LibraryElementKind::GlobalVarDeclarations(vars) => {
                for v in vars {
                    if v.qualifier == DeclarationQualifier::Retain {
                        if let Some(id) = v.identifier.symbolic_id() {
                            out.push(id.lower_case().clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Compile a whole project against the schedule stored in `tasks.toml`.
/// Thin wrapper over `compile_project_with_tasks` that reads the schedule
/// from disk — the natural entry point for "Run the project" / Deploy.
pub fn compile_project(store: &project::ProjectStore) -> Result<Container, BridgeError> {
    let tasks = store
        .read_tasks()
        .map_err(|e| BridgeError::Parse(format!("reading tasks.toml: {e}")))?
        .unwrap_or_default();
    compile_project_with_tasks(store, &tasks)
}

/// Like `compile_project` but returns metadata too. Run paths
/// (server `/api/run`, headless `ia2-runtime`) call this so they can
/// pass `retain_vars` into the spawn options.
pub fn compile_project_full(
    store: &project::ProjectStore,
) -> Result<(Container, ProgramMetadata), BridgeError> {
    let tasks = store
        .read_tasks()
        .map_err(|e| BridgeError::Parse(format!("reading tasks.toml: {e}")))?
        .unwrap_or_default();
    compile_project_with_tasks_full(store, &tasks)
}

/// Same as `compile_project` but takes an explicit `Tasks` instead of
/// reading `tasks.toml`. Lets the server layer compose ad-hoc schedules
/// (e.g. "Run just THIS POU once, ignoring the persisted schedule") for
/// the ProgramPane Run button without touching tasks.toml on disk.
///
/// POU files should NOT contain their own CONFIGURATION blocks — the
/// auto-migration step strips them on first open of legacy projects.
/// If any survive (e.g. user pasted one in by hand), they're stripped
/// here so the synthesized one wins without conflict.
pub fn compile_project_with_tasks(
    store: &project::ProjectStore,
    tasks: &project::Tasks,
) -> Result<Container, BridgeError> {
    compile_project_with_tasks_full(store, tasks).map(|(c, _)| c)
}

/// Like `compile_project_with_tasks`, but also returns the
/// `ProgramMetadata` (retain vars etc.) extracted from the parsed AST.
/// Used by the run path; existing callers that only care about
/// diagnostics keep using `compile_project_with_tasks`.
pub fn compile_project_with_tasks_full(
    store: &project::ProjectStore,
    tasks: &project::Tasks,
) -> Result<(Container, ProgramMetadata), BridgeError> {
    let combined = build_combined_project_source(store, tasks)?;
    tracing::debug!(
        len = combined.len(),
        "compile_project: combined source built"
    );
    compile_with_metadata(&combined)
}

/// Concatenate every POU's lowered ST + a synthesized CONFIGURATION
/// into a single compilation unit. Factored out of
/// `compile_project_with_tasks_full` so the two-stage compile +
/// metadata-extract pipeline (parse → codegen, then parse again for
/// retain) reuses the exact same input.
fn build_combined_project_source(
    store: &project::ProjectStore,
    tasks: &project::Tasks,
) -> Result<String, BridgeError> {
    let pou_paths = store
        .list_pou_paths()
        .map_err(|e| BridgeError::Parse(format!("listing pous: {e}")))?;

    if tasks.programs.is_empty() {
        return Err(BridgeError::Parse(
            "no PROGRAM instances to run — bind a PROGRAM to a task in the \
             Tasks pane, or click Run from a POU file's editor (which schedules \
             that PROGRAM ad-hoc for one run)"
                .into(),
        ));
    }

    let mut combined = String::new();
    for path in &pou_paths {
        let language = store
            .pou_file_language(path)
            .map_err(|e| BridgeError::Parse(format!("language for '{path}': {e}")))?;
        let source = store
            .read_pou_source(path)
            .map_err(|e| BridgeError::Parse(format!("reading pou '{path}': {e}")))?;
        let st = source_to_st(&source, language)?;
        let cleaned = strip_any_configuration(&st);
        combined.push_str(&cleaned);
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }
    combined.push_str(&synthesize_configuration(tasks));
    Ok(combined)
}

/// Compile a single POU source + synthesized CONFIGURATION. Used by the
/// ProgramPane's ad-hoc Run path so opening cascade_pid.st and clicking
/// Run actually runs cascade_pid in isolation — without ironplc's debug
/// section pulling in variables from PROGRAMs declared in *other* files
/// (which is what happens when `compile_project` concatenates everything).
///
/// `language` selects how `source` is interpreted: `St` → use the text
/// verbatim, `Ld` → transpile from JSON to ST first.
pub fn compile_isolated_source(
    source: &str,
    language: project::PouLanguage,
    tasks: &project::Tasks,
) -> Result<Container, BridgeError> {
    compile_isolated_source_full(source, language, tasks).map(|(c, _)| c)
}

/// Like `compile_isolated_source`, but returns the AST-derived
/// `ProgramMetadata` alongside the bytecode container.
pub fn compile_isolated_source_full(
    source: &str,
    language: project::PouLanguage,
    tasks: &project::Tasks,
) -> Result<(Container, ProgramMetadata), BridgeError> {
    if tasks.programs.is_empty() {
        return Err(BridgeError::Parse("no PROGRAM instance to run".into()));
    }
    let st = source_to_st(source, language)?;
    let mut combined = String::with_capacity(st.len() + 256);
    combined.push_str(&strip_any_configuration(&st));
    if !combined.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str(&synthesize_configuration(tasks));
    compile_with_metadata(&combined)
}

/// Lower an arbitrary POU source into ST, ready for ironplc to parse.
/// `St` is the identity. `Ld` parses the JSON via the LD schema and
/// runs the transpiler. Returns a descriptive parse error if the input
/// shape doesn't match the declared language.
fn source_to_st(source: &str, language: project::PouLanguage) -> Result<String, BridgeError> {
    match language {
        project::PouLanguage::St => Ok(source.to_string()),
        project::PouLanguage::Ld => {
            let prog: project::LdProgram = serde_json::from_str(source)
                .map_err(|e| BridgeError::Parse(format!("LD JSON parse: {e}")))?;
            ld_transpile::transpile_to_st(&prog)
        }
        project::PouLanguage::Fbd => {
            let prog: project::FbdProgram = serde_json::from_str(source)
                .map_err(|e| BridgeError::Parse(format!("FBD JSON parse: {e}")))?;
            fbd_transpile::transpile_to_st(&prog)
        }
        project::PouLanguage::Sfc => {
            let prog: project::SfcProgram = serde_json::from_str(source)
                .map_err(|e| BridgeError::Parse(format!("SFC JSON parse: {e}")))?;
            sfc_transpile::transpile_to_st(&prog)
        }
        other => Err(BridgeError::Parse(format!(
            "{other:?} not yet supported by the bridge"
        ))),
    }
}

/// Build a single CONFIGURATION block from the project's task / program
/// bindings. Currently emits one fixed RESOURCE — multi-RESOURCE projects
/// aren't supported yet (the IEC standard allows them; not a frequent need
/// for single-edge deployments).
fn synthesize_configuration(tasks: &project::Tasks) -> String {
    let mut s = String::new();
    s.push_str("CONFIGURATION config\n");
    s.push_str("    RESOURCE plc_res ON PLC\n");
    for t in &tasks.tasks {
        s.push_str(&format!(
            "        TASK {}(INTERVAL := T#{}ms, PRIORITY := {});\n",
            sanitise_ident(&t.name),
            t.interval_ms.max(1),
            t.priority,
        ));
    }
    for p in &tasks.programs {
        s.push_str(&format!(
            "        PROGRAM {} WITH {} : {};\n",
            sanitise_ident(&p.instance),
            sanitise_ident(&p.task),
            sanitise_ident(&p.program),
        ));
    }
    s.push_str("    END_RESOURCE\n");
    s.push_str("END_CONFIGURATION\n");
    s
}

/// IEC identifiers are letters/digits/underscore. The project tree allows
/// `/` in POU names (folder paths); use just the leaf when referring to
/// program / task / instance names in the generated IEC source.
fn sanitise_ident(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Find every `CONFIGURATION … END_CONFIGURATION` block in `source` and
/// remove it. Tolerant of case + whitespace; used to defend against POU
/// files that still carry a pre-migration scheduling block.
fn strip_any_configuration(source: &str) -> String {
    let lower = source.to_ascii_lowercase();
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    while let Some(start_rel) = lower[cursor..].find("configuration ") {
        let abs = cursor + start_rel;
        out.push_str(&source[cursor..abs]);
        let Some(end_rel) = lower[abs..].find("end_configuration") else {
            out.push_str(&source[abs..]);
            cursor = source.len();
            break;
        };
        let abs_end = abs + end_rel + "end_configuration".len();
        cursor = abs_end;
        if source.as_bytes().get(cursor).copied() == Some(b'\n') {
            cursor += 1;
        }
    }
    out.push_str(&source[cursor..]);
    out
}

/// One diagnostic in source-positioned form (1-indexed line/column for the
/// Monaco editor). Severity is "error" for now — ironplc emits a single
/// diagnostic stream without warning/info distinction at this layer.
///
/// For graphical-language POUs, `ld_location` / `fbd_location` / `sfc_location`
/// carry the resolved element the diagnostic originated from. Exactly one of
/// them is populated (or neither, for ST POUs / boilerplate lines).
///
/// **The "thick" diagnostic fields** — `context`, `related`, `explanation` —
/// come from `ironplc::Diagnostic`'s richer model. ironplc attaches
/// "describing" strings (`variable=foo`), secondary labels (`did you mean:
/// bar?`), and ships an RST doc per problem code. We expose all three so
/// human users and agents both see what to do next, not just "syntax error".
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CheckDiagnostic {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    /// Extra context fragments (e.g. `"variable=ghost"`). Comes from
    /// ironplc's `Diagnostic.described`. Empty for diagnostics without
    /// context (most syntax errors).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<String>,
    /// Secondary labels pointing at related source locations — the
    /// canonical "did you mean: counter?" / "first declared here"
    /// pattern. Empty for diagnostics without related info.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<DiagnosticRelated>,
    /// Embedded explanation from ironplc's problem-code documentation
    /// (RST body, title stripped). `None` if the code isn't in
    /// ironplc's registry — true for our synthetic
    /// LD-PARSE / FBD-TRANSPILE etc. codes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    /// For LD POUs only: which LD element this diagnostic originated
    /// from. `None` for ST / FBD / SFC POUs and for diagnostics on
    /// boilerplate lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ld_location: Option<ld_transpile::LdLocation>,
    /// For FBD POUs only: which block / output binding / variable
    /// declaration this diagnostic originated from. `None` for other
    /// languages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fbd_location: Option<fbd_transpile::FbdLocation>,
    /// For SFC POUs only: which step / action / transition / variable
    /// the diagnostic originated from. `None` for other languages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfc_location: Option<sfc_transpile::SfcLocation>,
}

/// One secondary label on a diagnostic. Both Monaco's
/// `relatedInformation` and our graphical-editor banners render these
/// as clickable jumps from the primary diagnostic to the related
/// source location — that's what makes "did you mean: foo?" useful
/// instead of just informative.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct DiagnosticRelated {
    pub message: String,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    /// Same dispatch as the parent diagnostic. The related label
    /// usually lives on the SAME source as the primary, but we still
    /// run it through the source map so jumps from the editor land in
    /// the right graphical element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ld_location: Option<ld_transpile::LdLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fbd_location: Option<fbd_transpile::FbdLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfc_location: Option<sfc_transpile::SfcLocation>,
}

/// Parse + analyse a Structured Text source and return all diagnostics
/// (syntax errors, type errors, undeclared identifiers, etc.). Does NOT run
/// codegen — this is the fast path for editor squiggles.
pub fn check(source: &str) -> Vec<CheckDiagnostic> {
    check_inner(source, SourceMapKind::None)
}

/// Like `check`, but for a graphical-language POU: parses the source,
/// transpiles to ST, runs the ironplc pipeline, then maps each
/// diagnostic's line back to the originating LD / FBD element.
/// ST-language POUs should still go through `check` — this entry
/// dispatches on the supplied `language`.
///
/// Returns a single synthetic diagnostic if the JSON itself doesn't
/// parse (so the editor still gets a squiggle pointing at the broken
/// document rather than silent failure).
pub fn check_pou_source(source: &str, language: project::PouLanguage) -> Vec<CheckDiagnostic> {
    match language {
        project::PouLanguage::St => check(source),
        project::PouLanguage::Ld => {
            let prog: project::LdProgram = match serde_json::from_str(source) {
                Ok(p) => p,
                Err(e) => return vec![synthetic_parse_diag("LD-PARSE", "LD", &e)],
            };
            let (st, map) = match ld_transpile::transpile_to_st_with_map(&prog) {
                Ok(pair) => pair,
                Err(e) => return vec![synthetic_transpile_diag("LD-TRANSPILE", &e)],
            };
            check_inner(&st, SourceMapKind::Ld(&map))
        }
        project::PouLanguage::Fbd => {
            let prog: project::FbdProgram = match serde_json::from_str(source) {
                Ok(p) => p,
                Err(e) => return vec![synthetic_parse_diag("FBD-PARSE", "FBD", &e)],
            };
            let (st, map) = match fbd_transpile::transpile_to_st_with_map(&prog) {
                Ok(pair) => pair,
                Err(e) => return vec![synthetic_transpile_diag("FBD-TRANSPILE", &e)],
            };
            check_inner(&st, SourceMapKind::Fbd(&map))
        }
        project::PouLanguage::Sfc => {
            let prog: project::SfcProgram = match serde_json::from_str(source) {
                Ok(p) => p,
                Err(e) => return vec![synthetic_parse_diag("SFC-PARSE", "SFC", &e)],
            };
            let (st, map) = match sfc_transpile::transpile_to_st_with_map(&prog) {
                Ok(pair) => pair,
                Err(e) => return vec![synthetic_transpile_diag("SFC-TRANSPILE", &e)],
            };
            check_inner(&st, SourceMapKind::Sfc(&map))
        }
        _ => Vec::new(),
    }
}

/// One of: no map, or a map for each graphical language. Used by
/// `check_inner` to route diagnostics through the right reverse-mapping
/// helper without growing the public API.
enum SourceMapKind<'a> {
    None,
    Ld(&'a ld_transpile::LdSourceMap),
    Fbd(&'a fbd_transpile::FbdSourceMap),
    Sfc(&'a sfc_transpile::SfcSourceMap),
}

fn synthetic_parse_diag(code: &str, language_name: &str, e: &serde_json::Error) -> CheckDiagnostic {
    CheckDiagnostic {
        severity: "error".into(),
        code: code.into(),
        message: format!("Invalid {language_name} JSON: {e}"),
        start_line: e.line() as u32,
        start_column: e.column() as u32,
        end_line: e.line() as u32,
        end_column: (e.column() as u32).saturating_add(1),
        context: vec![],
        related: vec![],
        explanation: None,
        ld_location: None,
        fbd_location: None,
        sfc_location: None,
    }
}

fn synthetic_transpile_diag(code: &str, e: &BridgeError) -> CheckDiagnostic {
    CheckDiagnostic {
        severity: "error".into(),
        code: code.into(),
        message: format!("{e:?}"),
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
    }
}

/// Shared implementation of `check` / `check_pou_source`. If `map` is
/// present, every produced diagnostic is annotated with the LD / FBD
/// element whose generated ST line was reported.
fn check_inner(source: &str, map: SourceMapKind<'_>) -> Vec<CheckDiagnostic> {
    let file_id = FileId::default();
    // `allow_empty_var_blocks` mirrors the ironplc CLI flag. POU templates
    // we ship intentionally start with empty VAR / VAR_INPUT / VAR_OUTPUT
    // blocks — those should compile, not error.
    let options = CompilerOptions {
        allow_empty_var_blocks: true,
        ..Default::default()
    };

    let library = match ironplc_parser::parse_program(source, &file_id, &options) {
        Ok(l) => l,
        Err(d) => return vec![diag_to_dto_with_map(&d, source, &map)],
    };

    let (_, context) = match ironplc_analyzer::stages::analyze(&[&library], &options) {
        Ok(t) => t,
        Err(ds) => {
            return ds
                .iter()
                .map(|d| diag_to_dto_with_map(d, source, &map))
                .collect()
        }
    };

    context
        .diagnostics()
        .iter()
        .map(|d| diag_to_dto_with_map(d, source, &map))
        .collect()
}

/// A single declared variable in a POU, surfaced to the frontend so it can
/// offer auto-complete in the IO Mapping form.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct VariableInfo {
    pub name: String,
    pub type_name: String,
    /// "local" | "input" | "output" | "in_out" | "external" | "global" | "access" | "temp"
    pub direction: String,
}

/// Parse an ST source and return every POU declaration found in it,
/// using the canonical `project::PouDecl` shape (name + type + language).
/// Returns an empty list if parsing fails — partial state isn't useful
/// to surface here; the LSP diagnostics endpoint is the right path for
/// "what's wrong with this source".
///
/// Dispatches on `language`:
///
/// - `St`  — runs the ironplc parser and emits one `PouDecl` per
///   top-level PROGRAM / FUNCTION_BLOCK / FUNCTION.
/// - `Ld`  — parses the `.ld.json` source and emits a single `PouDecl`
///   built from the `LdProgram`'s name + pou_type. LD files are
///   single-declaration by design (see `crates/project/src/ld.rs`).
/// - `Fbd` — same idea as LD but for `.fbd.json`.
/// - `Sfc` — same idea as LD but for `.sfc.json`.
/// - others — returns empty (no parser yet). The new-POU dialog
///   prevents these from existing on disk, but be defensive.
pub fn extract_pou_declarations(
    source: &str,
    language: project::PouLanguage,
) -> Vec<project::PouDecl> {
    use project::{PouDecl, PouLanguage, PouType};
    match language {
        PouLanguage::St => extract_st_declarations(source),
        PouLanguage::Fbd => {
            // Same tolerant-parse pattern as LD: malformed JSON yields
            // an empty decl list rather than blowing up
            // /api/project. Without this branch the IDE never
            // discovers that an `.fbd.json` POU is FBD-language and
            // dispatches the Monaco ST editor — which is wrong.
            match serde_json::from_str::<project::FbdProgram>(source) {
                Ok(prog) => vec![PouDecl {
                    name: prog.name,
                    type_: match prog.pou_type {
                        project::LdPouType::Program => PouType::Program,
                        project::LdPouType::FunctionBlock => PouType::FunctionBlock,
                    },
                    language: PouLanguage::Fbd,
                }],
                Err(e) => {
                    tracing::warn!(error = %e, "failed to parse FBD source for declarations");
                    vec![]
                }
            }
        }
        PouLanguage::Sfc => match serde_json::from_str::<project::SfcProgram>(source) {
            Ok(prog) => vec![PouDecl {
                name: prog.name,
                type_: match prog.pou_type {
                    project::LdPouType::Program => PouType::Program,
                    project::LdPouType::FunctionBlock => PouType::FunctionBlock,
                },
                language: PouLanguage::Sfc,
            }],
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse SFC source for declarations");
                vec![]
            }
        },
        PouLanguage::Ld => {
            // Tolerant: if the JSON is malformed (mid-edit save, say)
            // we just return no decls rather than blowing up the
            // entire /api/project response.
            match serde_json::from_str::<project::LdProgram>(source) {
                Ok(prog) => vec![PouDecl {
                    name: prog.name,
                    type_: match prog.pou_type {
                        project::LdPouType::Program => PouType::Program,
                        project::LdPouType::FunctionBlock => PouType::FunctionBlock,
                    },
                    language: PouLanguage::Ld,
                }],
                Err(e) => {
                    tracing::warn!(error = %e, "failed to parse LD source for declarations");
                    vec![]
                }
            }
        }
        _ => vec![],
    }
}

fn extract_st_declarations(source: &str) -> Vec<project::PouDecl> {
    use project::{PouDecl, PouLanguage, PouType};
    let file_id = FileId::default();
    let options = CompilerOptions {
        allow_empty_var_blocks: true,
        ..Default::default()
    };
    let Ok(library) = ironplc_parser::parse_program(source, &file_id, &options) else {
        return vec![];
    };
    let mut out = Vec::new();
    for element in &library.elements {
        let (name, type_) = match element {
            LibraryElementKind::ProgramDeclaration(p) => (p.name.to_string(), PouType::Program),
            LibraryElementKind::FunctionBlockDeclaration(fb) => {
                (fb.name.to_string(), PouType::FunctionBlock)
            }
            LibraryElementKind::FunctionDeclaration(f) => (f.name.to_string(), PouType::Function),
            _ => continue,
        };
        out.push(PouDecl {
            name,
            type_,
            language: PouLanguage::St,
        });
    }
    out
}

/// Language-agnostic symbol extraction. Same role as `extract_variables`
/// but works for ST / LD / FBD / SFC and adds FB instances (FBD blocks)
/// as synthetic VAR rows. Drives Monaco hover + variable completion
/// in the editor and the `cs symbols` CLI subcommand.
///
/// Returns `[]` for unsupported languages and for malformed sources —
/// callers shouldn't fall over when the user is mid-edit and the JSON
/// is briefly invalid.
pub fn extract_symbols(source: &str, language: project::PouLanguage) -> Vec<VariableInfo> {
    use project::PouLanguage;
    match language {
        PouLanguage::St => extract_variables(source),
        PouLanguage::Ld => match serde_json::from_str::<project::LdProgram>(source) {
            Ok(prog) => prog
                .variables
                .into_iter()
                .map(|v| VariableInfo {
                    name: v.name,
                    type_name: v.type_name,
                    direction: ld_section_str(v.section).into(),
                })
                .collect(),
            Err(_) => vec![],
        },
        PouLanguage::Fbd => match serde_json::from_str::<project::FbdProgram>(source) {
            Ok(prog) => {
                let mut out: Vec<VariableInfo> = prog
                    .variables
                    .iter()
                    .map(|v| VariableInfo {
                        name: v.name.clone(),
                        type_name: v.type_name.clone(),
                        direction: ld_section_str(v.section).into(),
                    })
                    .collect();
                // FB instances are first-class symbols too — Monaco's
                // hover on `myT1` should reveal `TON`, not pretend it
                // doesn't exist.
                for b in &prog.blocks {
                    out.push(VariableInfo {
                        name: b.instance.clone(),
                        type_name: b.fb_type.clone(),
                        direction: "fb_instance".into(),
                    });
                }
                out
            }
            Err(_) => vec![],
        },
        PouLanguage::Sfc => match serde_json::from_str::<project::SfcProgram>(source) {
            Ok(prog) => prog
                .variables
                .into_iter()
                .map(|v| VariableInfo {
                    name: v.name,
                    type_name: v.type_name,
                    direction: ld_section_str(v.section).into(),
                })
                .collect(),
            Err(_) => vec![],
        },
        _ => vec![],
    }
}

fn ld_section_str(s: project::LdVarSection) -> &'static str {
    match s {
        project::LdVarSection::Input => "input",
        project::LdVarSection::Output => "output",
        project::LdVarSection::Internal => "internal",
    }
}

/// Parse an ST source (no full compile required) and pull out the declared
/// variables of any PROGRAM or FUNCTION_BLOCK at the top level. Returns an
/// empty list if parsing fails — the frontend can fall back to free text.
pub fn extract_variables(source: &str) -> Vec<VariableInfo> {
    let file_id = FileId::default();
    // `allow_empty_var_blocks` mirrors the ironplc CLI flag. POU templates
    // we ship intentionally start with empty VAR / VAR_INPUT / VAR_OUTPUT
    // blocks — those should compile, not error.
    let options = CompilerOptions {
        allow_empty_var_blocks: true,
        ..Default::default()
    };
    let library = match ironplc_parser::parse_program(source, &file_id, &options) {
        Ok(l) => l,
        Err(_) => return vec![],
    };

    let mut out: Vec<VariableInfo> = Vec::new();
    for element in &library.elements {
        let vars = match element {
            LibraryElementKind::ProgramDeclaration(p) => Some(&p.variables),
            LibraryElementKind::FunctionBlockDeclaration(fb) => Some(&fb.variables),
            _ => None,
        };
        if let Some(vars) = vars {
            for v in vars {
                if let Some(info) = var_decl_to_info(v) {
                    out.push(info);
                }
            }
        }
    }
    out
}

fn var_decl_to_info(v: &VarDecl) -> Option<VariableInfo> {
    let name = v.identifier.symbolic_id()?.to_string();
    let type_name = init_type_name(&v.initializer);
    let direction = match v.var_type {
        VariableType::Var => "local",
        VariableType::VarTemp => "temp",
        VariableType::Input => "input",
        VariableType::Output => "output",
        VariableType::InOut => "in_out",
        VariableType::External => "external",
        VariableType::Global => "global",
        VariableType::Access => "access",
    }
    .to_string();
    Some(VariableInfo {
        name,
        type_name,
        direction,
    })
}

fn init_type_name(init: &InitialValueAssignmentKind) -> String {
    use InitialValueAssignmentKind::*;
    match init {
        Simple(s) => s.type_name.to_string(),
        String(s) => s.type_name().to_string(),
        FunctionBlock(fb) => fb.type_name.to_string(),
        LateResolvedType(t) => t.to_string(),
        EnumeratedType(e) => e.type_name.to_string(),
        Structure(s) => s.type_name.to_string(),
        Array(_) => "ARRAY".into(),
        Reference(_) => "REF_TO".into(),
        _ => "?".into(),
    }
}

/// Convert an ironplc diagnostic to our wire DTO, additionally looking
/// up the LD / FBD origin via `map` when present. Diagnostics on lines
/// that don't correspond to a graphical element (boilerplate, synthetic
/// temps) arrive with both `ld_location` and `fbd_location` = None,
/// which the editor renders as a generic file-level error rather than
/// a per-element squiggle.
fn diag_to_dto_with_map(d: &Diagnostic, source: &str, map: &SourceMapKind<'_>) -> CheckDiagnostic {
    let start = LineColumn::from_offset(source, d.primary.location.start);
    let end = LineColumn::from_offset(source, d.primary.location.end);
    let start_line = start.line + 1;
    let (ld_location, fbd_location, sfc_location) = lookup_locations(map, start_line as usize);

    // `described` is ironplc's structured context — "variable=foo",
    // "type=BOOL", etc. We pass it through verbatim; the editor /
    // CLI present it as one short line per entry under the primary
    // message.
    let context = d.described.clone();

    // Each secondary label resolves to a related-info entry. ironplc
    // emits them with byte offsets, so we run them through the same
    // (offset → line/col → optional graphical location) pipeline as
    // the primary label.
    let related: Vec<DiagnosticRelated> = d
        .secondary
        .iter()
        .map(|label| {
            let rs = LineColumn::from_offset(source, label.location.start);
            let re = LineColumn::from_offset(source, label.location.end);
            let rline = rs.line + 1;
            let (rl_loc, rf_loc, rs_loc) = lookup_locations(map, rline as usize);
            DiagnosticRelated {
                message: label.message.clone(),
                start_line: rline,
                start_column: rs.column + 1,
                end_line: re.line + 1,
                end_column: re.column + 1,
                ld_location: rl_loc,
                fbd_location: rf_loc,
                sfc_location: rs_loc,
            }
        })
        .collect();

    // Embedded RST doc — only present for ironplc's own problem codes
    // (P0001..P9999). Our synthetic codes (LD-PARSE etc.) won't match.
    let explanation = problem_docs::lookup_problem_explanation(&d.code);

    CheckDiagnostic {
        severity: "error".into(),
        code: d.code.clone(),
        message: d.description(),
        start_line,
        start_column: start.column + 1,
        end_line: end.line + 1,
        end_column: end.column + 1,
        context,
        related,
        explanation,
        ld_location,
        fbd_location,
        sfc_location,
    }
}

/// Resolve a 1-indexed ST line to its (ld, fbd, sfc) graphical
/// origin, using whichever source map is active. Used by both the
/// primary diagnostic and each related label.
fn lookup_locations(
    map: &SourceMapKind<'_>,
    line: usize,
) -> (
    Option<ld_transpile::LdLocation>,
    Option<fbd_transpile::FbdLocation>,
    Option<sfc_transpile::SfcLocation>,
) {
    match map {
        SourceMapKind::None => (None, None, None),
        SourceMapKind::Ld(m) => (m.lookup(line).cloned(), None, None),
        SourceMapKind::Fbd(m) => (None, m.lookup(line).cloned(), None),
        SourceMapKind::Sfc(m) => (None, None, m.lookup(line).cloned()),
    }
}

#[cfg(test)]
mod retain_tests {
    use super::compile_with_metadata;

    /// Wrapping the program in a CONFIGURATION is required for ironplc
    /// to accept the source; the synthesizer the bridge uses at runtime
    /// does the same thing.
    fn wrap(program: &str) -> String {
        format!(
            "{program}\n\
            CONFIGURATION cfg\n\
                RESOURCE res ON PLC\n\
                    TASK t1(INTERVAL := T#100ms, PRIORITY := 1);\n\
                    PROGRAM p1 WITH t1 : main;\n\
                END_RESOURCE\n\
            END_CONFIGURATION\n"
        )
    }

    #[test]
    fn retain_vars_are_extracted_from_program_var_retain_block() {
        let src = wrap(
            "PROGRAM main\n\
                VAR speed : INT := 0; END_VAR\n\
                VAR RETAIN setpoint : INT := 42; counter : DINT := 0; END_VAR\n\
                speed := setpoint;\n\
            END_PROGRAM",
        );
        let (_container, meta) = compile_with_metadata(&src).unwrap();
        assert_eq!(meta.retain_vars, vec!["counter", "setpoint"]);
    }

    #[test]
    fn programs_without_retain_blocks_produce_empty_metadata() {
        let src = wrap(
            "PROGRAM main\n\
                VAR x : INT := 1; END_VAR\n\
                x := x + 1;\n\
            END_PROGRAM",
        );
        let (_container, meta) = compile_with_metadata(&src).unwrap();
        assert!(meta.retain_vars.is_empty());
    }
}
