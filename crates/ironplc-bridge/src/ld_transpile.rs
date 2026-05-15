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
    LdCoilKind, LdComparator, LdNode, LdOperand, LdPouType, LdProgram, LdVarSection,
    LdVariable,
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

    // Pre-scan: any rung with >1 coil needs a BOOL temporary so we
    // evaluate the network exactly once and feed all coils from it.
    // ironplc requires every identifier to be declared in a VAR block
    // before use, so we collect the temp names up-front and emit them
    // as internal variables in the VAR section.
    let rung_temps: Vec<String> = prog
        .rungs
        .iter()
        .filter(|r| r.coils.len() > 1)
        .map(|r| format!("__rung_{}", sanitise_ident(&r.id)))
        .collect();

    let _ = writeln!(out, "{} {}", head, prog.name);
    write_variable_blocks(&mut out, &prog.variables, &rung_temps);
    out.push('\n');

    for (idx, rung) in prog.rungs.iter().enumerate() {
        emit_rung(&mut out, rung, idx)?;
    }

    let _ = writeln!(out, "{foot}");
    Ok(out)
}

fn write_variable_blocks(
    out: &mut String,
    vars: &[LdVariable],
    rung_temps: &[String],
) {
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
        // The VAR block also carries multi-coil rung temporaries
        // (`__rung_<id> : BOOL`). They're transpiler bookkeeping;
        // we keep them in the same block as the user's internal vars
        // because ironplc parses one VAR section per kind.
        if section == LdVarSection::Internal {
            for tmp in rung_temps {
                let _ = writeln!(out, "        {tmp} : BOOL;");
            }
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
        LdNode::Compare { left, cmp, right } => {
            // Compare block — bridges a numeric var into the boolean
            // network by emitting `(left CMP right)` as a parenthesised
            // ST sub-expression. ironplc parses these as normal
            // comparison operators against any numeric type, so the
            // user can mix INT, REAL, TIME literals etc. on either side.
            let lhs = render_operand(left)?;
            let rhs = render_operand(right)?;
            let op_str = match cmp {
                LdComparator::Eq => "=",
                LdComparator::Ne => "<>",
                LdComparator::Lt => "<",
                LdComparator::Le => "<=",
                LdComparator::Gt => ">",
                LdComparator::Ge => ">=",
            };
            format!("({lhs} {op_str} {rhs})")
        }
    })
}

fn render_operand(o: &LdOperand) -> Result<String, BridgeError> {
    Ok(match o {
        LdOperand::Var { name } => {
            if name.is_empty() {
                return Err(BridgeError::Parse(
                    "Compare operand has empty variable name".into(),
                ));
            }
            name.clone()
        }
        LdOperand::Literal { value } => {
            if value.is_empty() {
                return Err(BridgeError::Parse(
                    "Compare operand has empty literal".into(),
                ));
            }
            value.clone()
        }
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
    fn compare_block_emits_parenthesised_comparison() {
        // `temperature < 50.0`: a var on the left, a literal on the right.
        let node = LdNode::Compare {
            left: LdOperand::Var {
                name: "temperature".into(),
            },
            cmp: LdComparator::Lt,
            right: LdOperand::Literal {
                value: "50.0".into(),
            },
        };
        assert_eq!(render_node(&node).unwrap(), "(temperature < 50.0)");

        // Two-variable compare:
        let node2 = LdNode::Compare {
            left: LdOperand::Var { name: "pv".into() },
            cmp: LdComparator::Ge,
            right: LdOperand::Var { name: "sp".into() },
        };
        assert_eq!(render_node(&node2).unwrap(), "(pv >= sp)");

        // Inequality keyword maps to ST's `<>`:
        let node3 = LdNode::Compare {
            left: LdOperand::Var { name: "x".into() },
            cmp: LdComparator::Ne,
            right: LdOperand::Literal { value: "0".into() },
        };
        assert_eq!(render_node(&node3).unwrap(), "(x <> 0)");
    }

    #[test]
    fn compare_block_works_inside_and_chain() {
        // Real usage: a series AND with a compare block in the middle.
        //   start_btn AND (temperature < 50.0) AND NOT stop_btn
        let prog = LdProgram {
            name: "guard".into(),
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
                    name: "temperature".into(),
                    type_name: "REAL".into(),
                    section: LdVarSection::Input,
                    init: None,
                },
                LdVariable {
                    name: "heater".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Output,
                    init: None,
                },
            ],
            rungs: vec![LdRung {
                id: "guarded_heater".into(),
                label: None,
                logic: LdNode::And {
                    args: vec![
                        LdNode::Contact {
                            var: "start_btn".into(),
                            negated: false,
                        },
                        LdNode::Compare {
                            left: LdOperand::Var {
                                name: "temperature".into(),
                            },
                            cmp: LdComparator::Lt,
                            right: LdOperand::Literal {
                                value: "50.0".into(),
                            },
                        },
                        LdNode::Contact {
                            var: "stop_btn".into(),
                            negated: true,
                        },
                    ],
                },
                coils: vec![LdCoil {
                    var: "heater".into(),
                    kind: LdCoilKind::Standard,
                }],
            }],
        };
        let st = transpile_to_st(&prog).unwrap();
        assert!(
            st.contains("heater := (start_btn AND (temperature < 50.0) AND NOT stop_btn);"),
            "unexpected ST:\n{st}"
        );
    }

    #[test]
    fn multiple_coil_rung_declares_temporary_in_var_block() {
        // Regression: ironplc refuses ST that references an undeclared
        // identifier, so the `__rung_X` temp the transpiler synthesises
        // for multi-coil rungs must show up in the VAR section.
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
            ],
            rungs: vec![LdRung {
                id: "multi".into(),
                label: None,
                logic: LdNode::Contact {
                    var: "a".into(),
                    negated: false,
                },
                coils: vec![
                    LdCoil { var: "x".into(), kind: LdCoilKind::Standard },
                    LdCoil { var: "y".into(), kind: LdCoilKind::Standard },
                ],
            }],
        };
        let st = transpile_to_st(&prog).unwrap();
        // The temp must be declared in VAR before the rung uses it.
        let temp_decl_pos = st.find("__rung_multi : BOOL").expect("temp must be declared");
        let temp_use_pos = st.find("__rung_multi := a").expect("temp must be used");
        assert!(
            temp_decl_pos < temp_use_pos,
            "temp declaration must precede its use:\n{st}"
        );
        // And it must sit inside the internal VAR block, not VAR_INPUT/OUTPUT.
        let internal_var_pos = st.rfind("    VAR\n").expect("internal VAR block present");
        assert!(
            temp_decl_pos > internal_var_pos,
            "temp must be inside the internal VAR block, not the input/output ones"
        );
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
