//! Ladder Diagram → Structured Text transpiler.
//!
//! The pipeline for an LD POU is:
//!
//!   pous/<name>.ld.json   (canonical source)
//!     └── serde_json parse → project::LdProgram   (typed AST)
//!         └── transpile_to_st(&LdProgram) → String  (ST source)
//!             └── ironplc parser → DSL → codegen → bytecode
//!
//! Why ST as the intermediate: it's human-readable, agent-debuggable,
//! and reuses ironplc's existing pipeline end-to-end. See
//! MEMORY/graphical-languages.md for the full rationale and trade-off
//! against PLCopen XML / direct-DSL alternatives.
//!
//! Source-map: not yet emitted. The current implementation produces
//! ST whose lines map line-for-line to elements in the LD JSON
//! (one rung → roughly one statement / IF block), so diagnostics
//! coming back from ironplc with line numbers can be best-effort
//! mapped to rung IDs by counting transpiler emit order. Proper
//! source-map plumbing lands in a follow-up; the rung `id` field on
//! `LdRung` is the anchor.

use std::fmt::Write;

use project::{
    LdCoilKind, LdNode, LdPouType, LdProgram, LdVarSection, LdVariable,
};

use crate::errors::BridgeError;

/// Top-level entry: render an `LdProgram` to a complete ST source
/// string ready to feed `ironplc_bridge::compile` (or
/// `compile_isolated_source`).
pub fn transpile_to_st(prog: &LdProgram) -> Result<String, BridgeError> {
    if prog.name.is_empty() {
        return Err(BridgeError::Parse("LD program name is empty".into()));
    }
    if prog.rungs.is_empty() {
        return Err(BridgeError::Parse(format!(
            "LD program '{}' has no rungs",
            prog.name
        )));
    }

    let mut out = String::new();
    let (head, foot) = match prog.pou_type {
        LdPouType::Program => ("PROGRAM", "END_PROGRAM"),
        LdPouType::FunctionBlock => ("FUNCTION_BLOCK", "END_FUNCTION_BLOCK"),
    };

    let _ = writeln!(out, "{} {}", head, prog.name);
    write_variable_blocks(&mut out, &prog.variables);
    out.push('\n');

    for (idx, rung) in prog.rungs.iter().enumerate() {
        emit_rung(&mut out, rung, idx)?;
    }

    let _ = writeln!(out, "{foot}");
    Ok(out)
}

fn write_variable_blocks(out: &mut String, vars: &[LdVariable]) {
    // IEC requires VAR blocks in a specific order with no duplicates,
    // and ironplc's `allow_empty_var_blocks` already lets us emit
    // empty sections — so we always emit all three for shape stability.
    for section in [LdVarSection::Input, LdVarSection::Output, LdVarSection::Internal] {
        let header = match section {
            LdVarSection::Input => "VAR_INPUT",
            LdVarSection::Output => "VAR_OUTPUT",
            LdVarSection::Internal => "VAR",
        };
        let _ = writeln!(out, "    {header}");
        for v in vars.iter().filter(|v| v.section == section) {
            let init = v
                .init
                .as_ref()
                .map(|s| format!(" := {s}"))
                .unwrap_or_default();
            let _ = writeln!(out, "        {} : {}{};", v.name, v.type_name, init);
        }
        out.push_str("    END_VAR\n");
    }
}

fn emit_rung(
    out: &mut String,
    rung: &project::LdRung,
    idx: usize,
) -> Result<(), BridgeError> {
    if rung.coils.is_empty() {
        return Err(BridgeError::Parse(format!(
            "LD rung {} ({}) has no coils — a rung with no output is dead code",
            idx, rung.id
        )));
    }
    let logic = render_node(&rung.logic)?;
    if let Some(label) = &rung.label {
        let _ = writeln!(out, "    (* rung {}: {} *)", rung.id, label);
    } else {
        let _ = writeln!(out, "    (* rung {} *)", rung.id);
    }
    // Most rungs have one coil; if there are several, we evaluate
    // `logic` once via a temporary so re-running the network for each
    // coil can't produce inconsistent reads of mid-scan signals.
    // The temporary uses a stable identifier derived from the rung id
    // so name collisions across rungs are impossible.
    if rung.coils.len() == 1 {
        emit_coil(out, &rung.coils[0], &logic);
    } else {
        let tmp = format!("__rung_{}", sanitise_ident(&rung.id));
        let _ = writeln!(out, "    {tmp} := {logic};");
        for coil in &rung.coils {
            emit_coil(out, coil, &tmp);
        }
    }
    out.push('\n');
    Ok(())
}

fn emit_coil(out: &mut String, coil: &project::LdCoil, expr: &str) {
    match coil.kind {
        LdCoilKind::Standard => {
            let _ = writeln!(out, "    {} := {};", coil.var, expr);
        }
        LdCoilKind::Set => {
            let _ = writeln!(
                out,
                "    IF {expr} THEN {} := TRUE; END_IF;",
                coil.var
            );
        }
        LdCoilKind::Reset => {
            let _ = writeln!(
                out,
                "    IF {expr} THEN {} := FALSE; END_IF;",
                coil.var
            );
        }
    }
}

/// Render a boolean network into a parenthesised ST expression.
/// The recursion produces the obvious mapping:
///   contact{var}                → `var`            (or `NOT var` if negated)
///   and{args:[a,b,c]}           → `(a AND b AND c)`
///   or{args:[a,b]}              → `(a OR b)`
///   not{arg:a}                  → `(NOT a)`
///   const{value:true}           → `TRUE`
///
/// Empty AND/OR collapse to identity literals (`TRUE` / `FALSE`)
/// rather than erroring — saves UI clients from having to special-
/// case in-progress edits where a parallel branch is currently empty.
fn render_node(node: &LdNode) -> Result<String, BridgeError> {
    Ok(match node {
        LdNode::Contact { var, negated } => {
            if var.is_empty() {
                return Err(BridgeError::Parse(
                    "LD contact has empty variable name".into(),
                ));
            }
            if *negated {
                format!("NOT {var}")
            } else {
                var.clone()
            }
        }
        LdNode::And { args } => combine(args, "AND", "TRUE")?,
        LdNode::Or { args } => combine(args, "OR", "FALSE")?,
        LdNode::Not { arg } => format!("(NOT {})", render_node(arg)?),
        LdNode::Const { value } => if *value { "TRUE" } else { "FALSE" }.to_string(),
    })
}

fn combine(args: &[LdNode], op: &str, identity: &str) -> Result<String, BridgeError> {
    match args.len() {
        0 => Ok(identity.to_string()),
        1 => render_node(&args[0]),
        _ => {
            let mut parts = Vec::with_capacity(args.len());
            for a in args {
                parts.push(render_node(a)?);
            }
            Ok(format!("({})", parts.join(&format!(" {op} "))))
        }
    }
}

/// Borrowed from the synthesise-configuration code path in lib.rs —
/// turn an arbitrary rung id into something IEC allows as an
/// identifier (alphanumeric + underscore, leading char a letter).
/// Kept local so this module stays drop-in.
fn sanitise_ident(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 1);
    let mut chars = s.chars();
    if let Some(c) = chars.next() {
        if c.is_ascii_alphabetic() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    for c in chars {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

// =================================================================
//   Tests
// =================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use project::{LdCoil, LdRung, LdVariable};

    fn motor_seal_program() -> LdProgram {
        // Classic seal-in: pressing Start latches motor_run, which
        // remains latched until Stop is pressed.
        //   motor_run := Start OR (motor_run AND NOT Stop)
        LdProgram {
            name: "motor".into(),
            pou_type: LdPouType::Program,
            variables: vec![
                LdVariable {
                    name: "start_btn".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Input,
                    init: None,
                },
                LdVariable {
                    name: "stop_btn".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Input,
                    init: None,
                },
                LdVariable {
                    name: "motor_run".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Output,
                    init: Some("FALSE".into()),
                },
            ],
            rungs: vec![LdRung {
                id: "r0".into(),
                label: Some("motor seal-in".into()),
                logic: LdNode::Or {
                    args: vec![
                        LdNode::Contact {
                            var: "start_btn".into(),
                            negated: false,
                        },
                        LdNode::And {
                            args: vec![
                                LdNode::Contact {
                                    var: "motor_run".into(),
                                    negated: false,
                                },
                                LdNode::Contact {
                                    var: "stop_btn".into(),
                                    negated: true,
                                },
                            ],
                        },
                    ],
                },
                coils: vec![LdCoil {
                    var: "motor_run".into(),
                    kind: LdCoilKind::Standard,
                }],
            }],
        }
    }

    #[test]
    fn seal_in_transpiles_to_expected_st() {
        let st = transpile_to_st(&motor_seal_program()).unwrap();
        assert!(st.contains("PROGRAM motor"));
        assert!(st.contains("VAR_INPUT"));
        assert!(st.contains("start_btn : BOOL"));
        assert!(st.contains("motor_run : BOOL := FALSE"));
        // The full network: (start_btn OR (motor_run AND NOT stop_btn))
        assert!(
            st.contains("motor_run := (start_btn OR (motor_run AND NOT stop_btn));"),
            "unexpected ST:\n{st}"
        );
        assert!(st.contains("END_PROGRAM"));
    }

    #[test]
    fn negated_contact_becomes_not() {
        let st = transpile_to_st(&LdProgram {
            name: "p".into(),
            pou_type: LdPouType::Program,
            variables: vec![LdVariable {
                name: "x".into(),
                type_name: "BOOL".into(),
                section: LdVarSection::Internal,
                init: None,
            }],
            rungs: vec![LdRung {
                id: "r".into(),
                label: None,
                logic: LdNode::Contact {
                    var: "x".into(),
                    negated: true,
                },
                coils: vec![LdCoil {
                    var: "x".into(),
                    kind: LdCoilKind::Standard,
                }],
            }],
        })
        .unwrap();
        assert!(st.contains("x := NOT x;"));
    }

    #[test]
    fn set_and_reset_coils_emit_if_blocks() {
        let prog = LdProgram {
            name: "p".into(),
            pou_type: LdPouType::Program,
            variables: vec![
                LdVariable {
                    name: "trig".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Input,
                    init: None,
                },
                LdVariable {
                    name: "latched".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Output,
                    init: None,
                },
            ],
            rungs: vec![
                LdRung {
                    id: "set".into(),
                    label: None,
                    logic: LdNode::Contact {
                        var: "trig".into(),
                        negated: false,
                    },
                    coils: vec![LdCoil {
                        var: "latched".into(),
                        kind: LdCoilKind::Set,
                    }],
                },
                LdRung {
                    id: "rst".into(),
                    label: None,
                    logic: LdNode::Contact {
                        var: "trig".into(),
                        negated: true,
                    },
                    coils: vec![LdCoil {
                        var: "latched".into(),
                        kind: LdCoilKind::Reset,
                    }],
                },
            ],
        };
        let st = transpile_to_st(&prog).unwrap();
        assert!(st.contains("IF trig THEN latched := TRUE; END_IF;"));
        assert!(st.contains("IF NOT trig THEN latched := FALSE; END_IF;"));
    }

    #[test]
    fn empty_and_or_collapse_to_identity() {
        assert_eq!(render_node(&LdNode::And { args: vec![] }).unwrap(), "TRUE");
        assert_eq!(render_node(&LdNode::Or { args: vec![] }).unwrap(), "FALSE");
    }

    #[test]
    fn multiple_coils_use_temporary() {
        let prog = LdProgram {
            name: "p".into(),
            pou_type: LdPouType::Program,
            variables: vec![
                LdVariable {
                    name: "a".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Input,
                    init: None,
                },
                LdVariable {
                    name: "x".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Output,
                    init: None,
                },
                LdVariable {
                    name: "y".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Output,
                    init: None,
                },
                LdVariable {
                    name: "__rung_multi".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Internal,
                    init: None,
                },
            ],
            rungs: vec![LdRung {
                id: "multi".into(),
                label: None,
                logic: LdNode::Contact {
                    var: "a".into(),
                    negated: false,
                },
                coils: vec![
                    LdCoil {
                        var: "x".into(),
                        kind: LdCoilKind::Standard,
                    },
                    LdCoil {
                        var: "y".into(),
                        kind: LdCoilKind::Standard,
                    },
                ],
            }],
        };
        let st = transpile_to_st(&prog).unwrap();
        assert!(st.contains("__rung_multi := a;"));
        assert!(st.contains("x := __rung_multi;"));
        assert!(st.contains("y := __rung_multi;"));
    }
}
