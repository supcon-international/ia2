//! Wraps vendored ironplc parser + analyzer + codegen + VM into a single
//! `compile(source) -> Container` and `spawn(container) -> ProgramHandle` API
//! intended for downstream consumption by the server crate.

mod errors;
mod runtime;

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

/// Compile a whole project: concatenate every POU source, synthesize one
/// IEC `CONFIGURATION` block from `tasks.toml`, and hand the combined
/// text to `compile`.
///
/// POU files should NOT contain their own CONFIGURATION blocks — the
/// auto-migration step strips them on first open of legacy projects. If
/// any survive (e.g. user pasted one in by hand), they're stripped here
/// so the synthesized one wins without conflict.
pub fn compile_project(store: &project::ProjectStore) -> Result<Container, BridgeError> {
    let apps = store
        .list_applications()
        .map_err(|e| BridgeError::Parse(format!("listing applications: {e}")))?;
    let tasks = store
        .read_tasks()
        .map_err(|e| BridgeError::Parse(format!("reading tasks.toml: {e}")))?
        .unwrap_or_default();

    if tasks.programs.is_empty() {
        return Err(BridgeError::Parse(
            "no PROGRAM instances declared in tasks.toml — bind a PROGRAM \
             to a task in the Tasks pane before running"
                .into(),
        ));
    }

    let mut combined = String::new();
    for app in &apps {
        let cleaned = strip_any_configuration(&app.source);
        combined.push_str(&cleaned);
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }
    combined.push_str(&synthesize_configuration(&tasks));
    tracing::debug!(len = combined.len(), "compile_project: combined source built");
    compile(&combined)
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
}

/// Parse + analyse a Structured Text source and return all diagnostics
/// (syntax errors, type errors, undeclared identifiers, etc.). Does NOT run
/// codegen — this is the fast path for editor squiggles.
pub fn check(source: &str) -> Vec<CheckDiagnostic> {
    let file_id = FileId::default();
    // `allow_empty_var_blocks` mirrors the ironplc CLI flag. POU templates
    // we ship intentionally start with empty VAR / VAR_INPUT / VAR_OUTPUT
    // blocks — those should compile, not error.
    let mut options = CompilerOptions::default();
    options.allow_empty_var_blocks = true;

    let library = match ironplc_parser::parse_program(source, &file_id, &options) {
        Ok(l) => l,
        Err(d) => return vec![diag_to_dto(&d, source)],
    };

    let (_, context) = match ironplc_analyzer::stages::analyze(&[&library], &options) {
        Ok(t) => t,
        Err(ds) => return ds.iter().map(|d| diag_to_dto(d, source)).collect(),
    };

    context
        .diagnostics()
        .iter()
        .map(|d| diag_to_dto(d, source))
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

fn diag_to_dto(d: &Diagnostic, source: &str) -> CheckDiagnostic {
    let start = LineColumn::from_offset(source, d.primary.location.start);
    let end = LineColumn::from_offset(source, d.primary.location.end);
    CheckDiagnostic {
        severity: "error".into(),
        code: d.code.clone(),
        message: d.description(),
        start_line: start.line + 1,
        start_column: start.column + 1,
        end_line: end.line + 1,
        end_column: end.column + 1,
    }
}
