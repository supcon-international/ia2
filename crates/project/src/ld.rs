//! Ladder Diagram (LD) JSON schema.
//!
//! An LD POU lives on disk as `pous/<slug>.ld.json` and follows the
//! shape defined here. Loaded by the store, transpiled to ST by the
//! bridge, then compiled by ironplc. The transpilation pass is in
//! `crates/ironplc-bridge/src/ld_transpile.rs`.
//!
//! Design notes (see also MEMORY/graphical-languages.md):
//!
//! - JSON is the *canonical* storage format. No PLCopen XML round-trip.
//! - Each rung carries a recursive `LdNode` boolean expression
//!   (contact / coil / and / or / not). Series = AND, parallel = OR.
//! - Coil kinds: standard (`:=`), set (latch-on), reset (latch-off).
//!   Pulse / edge variants come later.
//! - Variables declared up-front in `variables`. The transpiler emits
//!   VAR / VAR_INPUT / VAR_OUTPUT blocks from that list — keeping the
//!   schema flat (just a section enum per var) instead of grouping by
//!   section in the JSON, which would be more nesting for agents to
//!   produce.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Top-level LD POU. One file = one POU. Multi-declaration files
/// (the ST convention of stuffing multiple PROGRAMs in one `.st`) are
/// explicitly NOT supported for graphical languages — graphical POUs
/// are visually one diagram, splitting them would invite cognitive
/// load that doesn't pay off.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LdProgram {
    /// IEC identifier — what tasks.toml binds. Must match the file
    /// slug for now (`pous/motor_seal.ld.json` → `name: "motor_seal"`).
    pub name: String,
    /// PROGRAM or FUNCTION_BLOCK. Functions are less natural in LD
    /// (no I/O contacts), so we don't expose them in the UI yet, but
    /// the field accepts them for future cross-language reuse.
    #[serde(rename = "pou_type")]
    pub pou_type: LdPouType,
    /// Inline variable declarations. Flat list with explicit section;
    /// transpiler groups them back into VAR_INPUT / VAR_OUTPUT / VAR.
    pub variables: Vec<LdVariable>,
    /// Ordered list of rungs. Transpiled top-to-bottom so a later
    /// rung's contacts see the assignments of earlier rungs in the
    /// same scan (this matches classical LD evaluation semantics).
    pub rungs: Vec<LdRung>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum LdPouType {
    Program,
    FunctionBlock,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum LdVarSection {
    /// VAR — internal, persists across scans.
    Internal,
    /// VAR_INPUT — read by this POU, written by caller.
    Input,
    /// VAR_OUTPUT — written by this POU, read by caller.
    Output,
}

/// One variable. Type is a string so agents can declare any IEC type
/// (BOOL / INT / TIME / etc.) without us needing to enumerate.
/// Validation happens at transpile-time when ironplc parses the
/// generated ST.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LdVariable {
    pub name: String,
    /// IEC type name — e.g. "BOOL", "INT", "TIME", "REAL".
    #[serde(rename = "type")]
    pub type_name: String,
    pub section: LdVarSection,
    /// Optional initialiser literal (e.g. `"FALSE"`, `"T#100ms"`, `"42"`).
    /// Passed verbatim to the ST `:= <init>` clause. None → omit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LdRung {
    /// Stable identifier so the IDE / source-map can reference a rung
    /// across edits. Generated client-side; the transpiler doesn't
    /// look at it beyond echoing it into the source map.
    pub id: String,
    /// Optional human-readable label rendered above the rung.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Boolean network feeding the coil. Free-form recursive AND/OR/NOT
    /// of contacts.
    pub logic: LdNode,
    /// Coil(s) driven by `logic`. Most rungs have one coil; an array
    /// lets a single network drive multiple outputs (a common
    /// real-world idiom — "if EmergencyOff then disable everything").
    pub coils: Vec<LdCoil>,
}

/// Boolean expression tree. The transpiler turns this into a parenthesised
/// ST expression: `and { args: [a, b] }` → `(a AND b)`.
///
/// Why a tree, not a 2-D grid of contacts: a tree matches the recursive
/// "series / parallel" structure operators actually compose with. The
/// renderer projects it to a 2-D layout deterministically (series →
/// horizontal, parallel → stacked vertically), so authors think in
/// expressions and the visualisation falls out.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LdNode {
    /// Normally-open contact (passes when `var` is TRUE).
    /// `negated: true` flips it to a normally-closed contact (passes
    /// when `var` is FALSE).
    Contact {
        var: String,
        #[serde(default)]
        negated: bool,
    },
    /// Series — all branches must conduct.
    And { args: Vec<LdNode> },
    /// Parallel — at least one branch must conduct.
    Or { args: Vec<LdNode> },
    /// Inverter wrapping any sub-expression.
    Not { arg: Box<LdNode> },
    /// Constant rail — `value: true` is always-passing, useful for
    /// unconditional outputs. `value: false` is rare but exposed for
    /// completeness.
    Const { value: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LdCoil {
    pub var: String,
    pub kind: LdCoilKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum LdCoilKind {
    /// `var := <logic>;` — assigned every scan.
    Standard,
    /// Set / latch — `IF <logic> THEN var := TRUE; END_IF;`.
    Set,
    /// Reset / unlatch — `IF <logic> THEN var := FALSE; END_IF;`.
    Reset,
}

// =================================================================
//   Tiny smoke test that the schema round-trips through serde_json.
//   Real transpiler tests live in the bridge crate where they can
//   verify generated ST as well.
// =================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_minimal_program() {
        let prog = LdProgram {
            name: "motor".into(),
            pou_type: LdPouType::Program,
            variables: vec![LdVariable {
                name: "start_btn".into(),
                type_name: "BOOL".into(),
                section: LdVarSection::Input,
                init: None,
            }],
            rungs: vec![LdRung {
                id: "r0".into(),
                label: Some("first rung".into()),
                logic: LdNode::Contact {
                    var: "start_btn".into(),
                    negated: false,
                },
                coils: vec![LdCoil {
                    var: "motor_run".into(),
                    kind: LdCoilKind::Standard,
                }],
            }],
        };
        let json = serde_json::to_string_pretty(&prog).unwrap();
        let back: LdProgram = serde_json::from_str(&json).unwrap();
        assert_eq!(back.rungs.len(), 1);
        assert_eq!(back.rungs[0].coils[0].var, "motor_run");
    }

    #[test]
    fn nested_and_or_not_round_trips() {
        let n: LdNode = serde_json::from_str(
            r#"{"op":"and","args":[
                {"op":"contact","var":"a"},
                {"op":"or","args":[
                    {"op":"contact","var":"b","negated":true},
                    {"op":"not","arg":{"op":"contact","var":"c"}}
                ]}
            ]}"#,
        )
        .unwrap();
        match &n {
            LdNode::And { args } => assert_eq!(args.len(), 2),
            _ => panic!("expected And at root, got {:?}", n),
        }
    }
}
