//! Wraps vendored ironplc parser + analyzer + codegen + VM into a single
//! `compile(source) -> Container` and `spawn(container) -> ProgramHandle` API
//! intended for downstream consumption by the server crate.

mod errors;
mod fbd_transpile;
mod ld_transpile;
mod runtime;

pub use ld_transpile::transpile_to_st as transpile_ld_to_st;
pub use ld_transpile::{
    transpile_to_st_with_map as transpile_ld_to_st_with_map, LdLocation, LdSourceMap,
};
pub use fbd_transpile::transpile_to_st as transpile_fbd_to_st;
pub use fbd_transpile::{
    transpile_to_st_with_map as transpile_fbd_to_st_with_map, FbdLocation, FbdSourceMap,
};

pub use errors::BridgeError;
pub use runtime::{DeviceSpec, ProgramHandle, RuntimeWriteError, VarSnapshot, VarValue, spawn};


use ironplc_container::Container;
use ironplc_dsl::common::{
    InitialValueAssignmentKind, LibraryElementKind, VarDecl, VariableType,
};
use ironplc_dsl::core::FileId;
use ironplc_dsl::diagnostic::{Diagnostic, LineColumn};
use ironplc_parser::options::CompilerOptions;
use serde::Serialize;
use ts_rs::TS;

/// Compile an IEC 61131-3 Structured Text source string into an executable
/// ironplc bytecode `Container`. Uses dialect Ed2 with no vendor extensions.
pub fn compile(source: &str) -> Result<Container, BridgeError> {
    let file_id = FileId::default();
    // `allow_empty_var_blocks` mirrors the ironplc CLI flag. POU templates
    // we ship intentionally start with empty VAR / VAR_INPUT / VAR_OUTPUT
    // blocks — those should compile, not error.
    let mut options = CompilerOptions::default();
    options.allow_empty_var_blocks = true;

    let library = ironplc_parser::parse_program(source, &file_id, &options)
        .map_err(|d| BridgeError::Parse(format!("{d:?}")))?;

    let (analyzed, context) = ironplc_analyzer::stages::analyze(&[&library], &options)
        .map_err(|ds| BridgeError::Analyze(format!("{ds:?}")))?;

    if context.has_diagnostics() {
        return Err(BridgeError::Analyze(format!("{:?}", context.diagnostics())));
    }

    let codegen_options = ironplc_codegen::CodegenOptions::default();
    let container = ironplc_codegen::compile(&analyzed, &context, &codegen_options)
        .map_err(|d| BridgeError::Codegen(format!("{d:?}")))?;

    Ok(container)
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
    tracing::debug!(len = combined.len(), "compile_project: combined source built");
    compile(&combined)
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
    if tasks.programs.is_empty() {
        return Err(BridgeError::Parse(
            "no PROGRAM instance to run".into(),
        ));
    }
    let st = source_to_st(source, language)?;
    let mut combined = String::with_capacity(st.len() + 256);
    combined.push_str(&strip_any_configuration(&st));
    if !combined.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str(&synthesize_configuration(tasks));
    compile(&combined)
}

/// Lower an arbitrary POU source into ST, ready for ironplc to parse.
/// `St` is the identity. `Ld` parses the JSON via the LD schema and
/// runs the transpiler. Returns a descriptive parse error if the input
/// shape doesn't match the declared language.
fn source_to_st(
    source: &str,
    language: project::PouLanguage,
) -> Result<String, BridgeError> {
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
/// For graphical-language POUs, `ld_location` / `fbd_location` carry the
/// resolved element the diagnostic originated from. Exactly one of them
/// is populated (or neither, for ST POUs / boilerplate lines).
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
    /// For LD POUs only: which LD element this diagnostic originated
    /// from. `None` for ST / FBD POUs and for diagnostics on
    /// boilerplate lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ld_location: Option<ld_transpile::LdLocation>,
    /// For FBD POUs only: which block / output binding / variable
    /// declaration this diagnostic originated from. `None` for ST / LD
    /// POUs and for diagnostics on boilerplate lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fbd_location: Option<fbd_transpile::FbdLocation>,
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
pub fn check_pou_source(
    source: &str,
    language: project::PouLanguage,
) -> Vec<CheckDiagnostic> {
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
        _ => Vec::new(),
    }
}

/// One of: no map, an LD map, an FBD map. Used by `check_inner` to
/// route diagnostics through the right reverse-mapping helper without
/// growing the public API.
enum SourceMapKind<'a> {
    None,
    Ld(&'a ld_transpile::LdSourceMap),
    Fbd(&'a fbd_transpile::FbdSourceMap),
}

fn synthetic_parse_diag(
    code: &str,
    language_name: &str,
    e: &serde_json::Error,
) -> CheckDiagnostic {
    CheckDiagnostic {
        severity: "error".into(),
        code: code.into(),
        message: format!("Invalid {language_name} JSON: {e}"),
        start_line: e.line() as u32,
        start_column: e.column() as u32,
        end_line: e.line() as u32,
        end_column: (e.column() as u32).saturating_add(1),
        ld_location: None,
        fbd_location: None,
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
        ld_location: None,
        fbd_location: None,
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
    let mut options = CompilerOptions::default();
    options.allow_empty_var_blocks = true;

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
    let mut options = CompilerOptions::default();
    options.allow_empty_var_blocks = true;
    let Ok(library) = ironplc_parser::parse_program(source, &file_id, &options) else {
        return vec![];
    };
    let mut out = Vec::new();
    for element in &library.elements {
        let (name, type_) = match element {
            LibraryElementKind::ProgramDeclaration(p) => {
                (p.name.to_string(), PouType::Program)
            }
            LibraryElementKind::FunctionBlockDeclaration(fb) => {
                (fb.name.to_string(), PouType::FunctionBlock)
            }
            LibraryElementKind::FunctionDeclaration(f) => {
                (f.name.to_string(), PouType::Function)
            }
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

/// Parse an ST source (no full compile required) and pull out the declared
/// variables of any PROGRAM or FUNCTION_BLOCK at the top level. Returns an
/// empty list if parsing fails — the frontend can fall back to free text.
pub fn extract_variables(source: &str) -> Vec<VariableInfo> {
    let file_id = FileId::default();
    // `allow_empty_var_blocks` mirrors the ironplc CLI flag. POU templates
    // we ship intentionally start with empty VAR / VAR_INPUT / VAR_OUTPUT
    // blocks — those should compile, not error.
    let mut options = CompilerOptions::default();
    options.allow_empty_var_blocks = true;
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
fn diag_to_dto_with_map(
    d: &Diagnostic,
    source: &str,
    map: &SourceMapKind<'_>,
) -> CheckDiagnostic {
    let start = LineColumn::from_offset(source, d.primary.location.start);
    let end = LineColumn::from_offset(source, d.primary.location.end);
    let start_line = start.line + 1;
    let (ld_location, fbd_location) = match map {
        SourceMapKind::None => (None, None),
        SourceMapKind::Ld(m) => (m.lookup(start_line as usize).cloned(), None),
        SourceMapKind::Fbd(m) => (None, m.lookup(start_line as usize).cloned()),
    };
    CheckDiagnostic {
        severity: "error".into(),
        code: d.code.clone(),
        message: d.description(),
        start_line,
        start_column: start.column + 1,
        end_line: end.line + 1,
        end_column: end.column + 1,
        ld_location,
        fbd_location,
    }
}
