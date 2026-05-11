//! Wraps vendored ironplc parser + analyzer + codegen + VM into a single
//! `compile(source) -> Container` and `spawn(container) -> ProgramHandle` API
//! intended for downstream consumption by the server crate.

mod errors;
mod runtime;

pub use errors::BridgeError;
pub use runtime::{ProgramHandle, VarSnapshot, VarValue, spawn};

use ironplc_container::Container;
use ironplc_dsl::core::FileId;
use ironplc_parser::options::CompilerOptions;

/// Compile an IEC 61131-3 Structured Text source string into an executable
/// ironplc bytecode `Container`. Uses dialect Ed2 with no vendor extensions.
pub fn compile(source: &str) -> Result<Container, BridgeError> {
    let file_id = FileId::default();
    let options = CompilerOptions::default();

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
