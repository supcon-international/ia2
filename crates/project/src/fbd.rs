//! Function Block Diagram (FBD) JSON schema.
//!
//! Stored on disk as `pous/<slug>.fbd.json`. Loaded by the store,
//! transpiled to ST by `crates/ironplc-bridge/src/fbd_transpile.rs`,
//! then compiled by ironplc.
//!
//! Design notes (see also MEMORY/graphical-languages.md § FBD):
//!
//! - **Blocks + wires**, no boolean tree. Each block is a function
//!   block instance (TON, CTU, R_TRIG, …, or a user-defined FB
//!   declared in another POU); each input pin gets one binding —
//!   variable reference, inline literal, or a wire from another
//!   block's output pin.
//! - Output bindings are a separate list at the top — they wire a
//!   block's output to one of the POU's `VAR_OUTPUT` variables.
//! - We reuse `LdVariable` / `LdVarSection` for variable declarations.
//!   The schemas are identical (IEC 61131-3 VAR syntax is the same
//!   regardless of body language); duplicating would just create
//!   maintenance drift.
//! - `position` on each block is optional — agents can author
//!   FBD JSON without thinking about layout. The renderer auto-lays
//!   out via dagre when positions are absent; if a user drags blocks
//!   in the GUI we persist back into the JSON.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ld::{LdPouType, LdVariable};

/// Top-level FBD POU.
///
/// One file = one POU (same rule as LD; multi-POU files don't
/// generalise to graphical languages).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FbdProgram {
    /// IEC identifier — what tasks.toml binds.
    pub name: String,
    /// PROGRAM or FUNCTION_BLOCK. Reuses `LdPouType` because the
    /// distinction is identical across graphical languages.
    #[serde(rename = "pou_type")]
    pub pou_type: LdPouType,
    /// Inline variable declarations. Same `LdVariable` schema as LD
    /// (see module docs).
    pub variables: Vec<LdVariable>,
    /// Function block instances making up the diagram. Order is
    /// **authoring** order, not execution order — the transpiler
    /// topologically sorts on input dependencies before emitting
    /// calls.
    pub blocks: Vec<FbdBlock>,
    /// Wires from block outputs to the POU's external output
    /// variables. A `VAR_OUTPUT` variable can be driven by exactly
    /// one block pin; binding the same variable twice is an error.
    /// `VAR_OUTPUT` variables not bound here are assigned via
    /// regular ST elsewhere or left at their initial value.
    ///
    /// **Always serialized**, even when empty — the TS-generated
    /// frontend type marks this field as required, so omitting it
    /// turns into `undefined.length` at render time (caught a real
    /// crash on the agent's first `POST /api/pous` with the seeded
    /// empty-outputs template). `#[serde(default)]` still covers
    /// older files on disk that pre-date this change.
    #[serde(default)]
    pub outputs: Vec<FbdOutputBinding>,
}

/// One function block instance on the diagram.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FbdBlock {
    /// Stable identifier — referenced by wires (`FbdInputSource::Block`)
    /// and by source-map locations. Must be unique within the POU.
    /// Distinct from `instance` so the user can rename the FB instance
    /// without invalidating every wire's source-end reference.
    pub id: String,
    /// IEC type name — `"TON"`, `"CTU"`, or any FB declared elsewhere
    /// in the project. The transpiler emits `<instance> : <fb_type>;`
    /// in the internal VAR block; ironplc resolves the type at
    /// compile time.
    #[serde(rename = "fb_type")]
    pub fb_type: String,
    /// FB instance variable name. The transpiler declares it
    /// automatically — users don't add it to `variables`. Must be a
    /// valid IEC identifier and unique within the POU.
    pub instance: String,
    /// Pin bindings in authoring order. Pins that aren't bound here
    /// fall back to ironplc's defaults (FALSE / 0 / T#0ms). Always
    /// serialized (see `FbdProgram::outputs` for the rationale).
    #[serde(default)]
    pub inputs: Vec<FbdInputBinding>,
    /// Optional render position. Authoring-time hint only — the
    /// transpiler ignores it. Present when the user drags a block in
    /// the GUI; absent when an agent writes the JSON directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<FbdPosition>,
}

/// One pin binding on an FbdBlock.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FbdInputBinding {
    /// Pin name on the FB — `"IN"`, `"PT"`, `"CU"`, etc. Case
    /// preserved verbatim into the generated ST.
    pub pin: String,
    /// What feeds this pin.
    pub value: FbdInputSource,
}

/// Source of a pin's value.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FbdInputSource {
    /// Reference to one of the POU's variables (input, output, or
    /// internal). The transpiler emits the variable name verbatim;
    /// ironplc enforces type compatibility.
    Var { name: String },
    /// Inline literal — `"TRUE"`, `"T#3s"`, `"42"`. Passed through
    /// verbatim. Quoting / escaping is the author's problem.
    Literal { value: String },
    /// Wire from another block's output pin. Creates a dependency
    /// edge for topological sort. Cycles are forbidden in FBD (CFC
    /// allows them with explicit feedback markers — out of scope for
    /// the MVP).
    Block {
        /// Source block's `id` (NOT its instance name).
        block_id: String,
        /// Source block's output pin name.
        pin: String,
    },
}

/// Wire from a block output to a POU output variable.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FbdOutputBinding {
    /// One of the POU's `VAR_OUTPUT` variables.
    pub variable: String,
    /// Source block's `id`.
    pub from_block: String,
    /// Source block's output pin.
    pub from_pin: String,
}

/// 2-D layout hint for a block. Coordinates are in renderer units
/// (pixels by convention); the renderer respects them when present
/// and falls back to dagre auto-layout when not.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FbdPosition {
    pub x: f32,
    pub y: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ld::LdVarSection;

    #[test]
    fn round_trips_minimal_fbd() {
        let prog = FbdProgram {
            name: "demo".into(),
            pou_type: LdPouType::Program,
            variables: vec![LdVariable {
                name: "btn".into(),
                type_name: "BOOL".into(),
                section: LdVarSection::Input,
                init: None,
            }],
            blocks: vec![FbdBlock {
                id: "b0".into(),
                fb_type: "TON".into(),
                instance: "myT".into(),
                inputs: vec![
                    FbdInputBinding {
                        pin: "IN".into(),
                        value: FbdInputSource::Var { name: "btn".into() },
                    },
                    FbdInputBinding {
                        pin: "PT".into(),
                        value: FbdInputSource::Literal {
                            value: "T#1s".into(),
                        },
                    },
                ],
                position: None,
            }],
            outputs: vec![],
        };
        let json = serde_json::to_string_pretty(&prog).unwrap();
        let back: FbdProgram = serde_json::from_str(&json).unwrap();
        assert_eq!(back.blocks.len(), 1);
        assert_eq!(back.blocks[0].instance, "myT");
    }

    #[test]
    fn wire_input_source_parses() {
        let raw = r#"{"kind":"block","block_id":"b0","pin":"Q"}"#;
        let src: FbdInputSource = serde_json::from_str(raw).unwrap();
        match src {
            FbdInputSource::Block { block_id, pin } => {
                assert_eq!(block_id, "b0");
                assert_eq!(pin, "Q");
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }
}
