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

use std::collections::BTreeMap;
use std::fmt::Write;

use project::{
    LdCoilKind, LdComparator, LdFbInput, LdNode, LdOperand, LdPouType, LdProgram, LdRung,
    LdVarSection, LdVariable,
};

use crate::errors::BridgeError;

/// `(instance_name, fb_type)`. Stable across iterations because we
/// build it from a deterministic walk of the program.
type FbInstanceTable = BTreeMap<String, String>;

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

    // Pre-scan: collect every FB instance referenced from any rung's
    // logic tree. Each instance becomes a `name : fb_type;` line in
    // the internal VAR block. Conflicts (same instance name, different
    // FB types) error early — silently picking one would lead to a
    // very confusing ironplc diagnostic later.
    let fb_instances = collect_fb_instances(&prog.rungs)?;

    let _ = writeln!(out, "{} {}", head, prog.name);
    write_variable_blocks(&mut out, &prog.variables, &rung_temps, &fb_instances);
    out.push('\n');

    for (idx, rung) in prog.rungs.iter().enumerate() {
        emit_rung(&mut out, rung, idx)?;
    }

    let _ = writeln!(out, "{foot}");
    Ok(out)
}

/// Walk all rung logic trees, gathering every `FbCall` instance. Errors
/// if the same instance name appears with two different FB types.
fn collect_fb_instances(rungs: &[LdRung]) -> Result<FbInstanceTable, BridgeError> {
    let mut table = FbInstanceTable::new();
    for rung in rungs {
        walk_for_fb_calls(&rung.logic, &mut |instance, fb_type| {
            if let Some(prev) = table.get(instance) {
                if prev != fb_type {
                    return Err(BridgeError::Parse(format!(
                        "FB instance '{instance}' declared with conflicting types '{prev}' and '{fb_type}'"
                    )));
                }
            } else {
                table.insert(instance.to_string(), fb_type.to_string());
            }
            Ok(())
        })?;
    }
    Ok(table)
}

/// Tree walker that calls `visit(instance, fb_type)` for every
/// `FbCall` node encountered.
fn walk_for_fb_calls(
    node: &LdNode,
    visit: &mut dyn FnMut(&str, &str) -> Result<(), BridgeError>,
) -> Result<(), BridgeError> {
    match node {
        LdNode::Contact { .. } | LdNode::Const { .. } | LdNode::Compare { .. } => Ok(()),
        LdNode::And { args } | LdNode::Or { args } => {
            for a in args {
                walk_for_fb_calls(a, visit)?;
            }
            Ok(())
        }
        LdNode::Not { arg } => walk_for_fb_calls(arg, visit),
        LdNode::FbCall {
            instance, fb_type, ..
        } => {
            if instance.is_empty() {
                return Err(BridgeError::Parse(
                    "FB call has empty instance name".into(),
                ));
            }
            if fb_type.is_empty() {
                return Err(BridgeError::Parse(format!(
                    "FB call '{instance}' has empty fb_type"
                )));
            }
            visit(instance, fb_type)
        }
    }
}

fn write_variable_blocks(
    out: &mut String,
    vars: &[LdVariable],
    rung_temps: &[String],
    fb_instances: &FbInstanceTable,
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
        // The VAR block also carries (a) multi-coil rung temporaries
        // (`__rung_<id> : BOOL`) and (b) FB instances synthesised
        // from FbCall nodes (`myT1 : TON;`). They're transpiler
        // bookkeeping; we keep them in the same block as the user's
        // internal vars because ironplc parses one VAR section per
        // kind.
        if section == LdVarSection::Internal {
            for tmp in rung_temps {
                let _ = writeln!(out, "        {tmp} : BOOL;");
            }
            for (instance, fb_type) in fb_instances {
                let _ = writeln!(out, "        {instance} : {fb_type};");
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

    // Emit FB call statements for any FbCall nodes in this rung's
    // logic tree, BEFORE the coil assignment. ironplc's call-by-name
    // syntax is `inst(PIN := value, PIN := value);`. Each unique
    // instance is called once per rung (in source order); subsequent
    // references in the same rung read its output pin via dot syntax.
    //
    // Why once per rung (not once per POU): the inputs may depend on
    // values written by earlier rungs in the same scan, and we want
    // the FB to see the most recent values. Calling more than once
    // is also fine — for edge detectors the second call sees CLK
    // unchanged so it's a no-op.
    emit_fb_calls_for_rung(out, &rung.logic)?;

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

/// Walk the rung's logic tree in source order and emit one
/// `instance(PIN := value, ...);` line per unique FbCall instance.
/// De-duplicates so each instance is called at most once per rung.
fn emit_fb_calls_for_rung(out: &mut String, logic: &LdNode) -> Result<(), BridgeError> {
    let mut seen: Vec<String> = Vec::new();
    let mut emit = |node: &LdNode| -> Result<(), BridgeError> {
        if let LdNode::FbCall {
            instance, inputs, ..
        } = node
        {
            if !seen.iter().any(|s| s == instance) {
                seen.push(instance.clone());
                let args = render_fb_inputs(inputs)?;
                let _ = writeln!(out, "    {instance}({args});");
            }
        }
        Ok(())
    };
    walk_in_order(logic, &mut emit)?;
    Ok(())
}

/// In-order traversal that calls `visit` on every node. Used by
/// `emit_fb_calls_for_rung` so the call order matches authoring order.
fn walk_in_order(
    node: &LdNode,
    visit: &mut dyn FnMut(&LdNode) -> Result<(), BridgeError>,
) -> Result<(), BridgeError> {
    visit(node)?;
    match node {
        LdNode::Contact { .. } | LdNode::Const { .. } | LdNode::Compare { .. } | LdNode::FbCall { .. } => Ok(()),
        LdNode::And { args } | LdNode::Or { args } => {
            for a in args {
                walk_in_order(a, visit)?;
            }
            Ok(())
        }
        LdNode::Not { arg } => walk_in_order(arg, visit),
    }
}

/// Format an FB's input pins as a comma-separated `PIN := value` list
/// ready to drop inside an `inst(...)` call.
fn render_fb_inputs(inputs: &[LdFbInput]) -> Result<String, BridgeError> {
    let mut parts = Vec::with_capacity(inputs.len());
    for inp in inputs {
        if inp.pin.is_empty() {
            return Err(BridgeError::Parse("FB call has empty pin name".into()));
        }
        let value = render_operand(&inp.value)?;
        parts.push(format!("{} := {}", inp.pin, value));
    }
    Ok(parts.join(", "))
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
        LdNode::FbCall {
            instance,
            output_pin,
            ..
        } => {
            // The actual call statement (`instance(IN := ..., ...);`)
            // was already emitted by `emit_fb_calls_for_rung` before
            // we got here. In the boolean expression, an FbCall node
            // contributes its chosen output pin's value: `instance.Q`,
            // `instance.QU`, etc. The dot-access syntax is standard
            // IEC 61131-3 — ironplc parses it as a member reference
            // on the FB instance.
            if instance.is_empty() {
                return Err(BridgeError::Parse(
                    "FB call has empty instance name".into(),
                ));
            }
            if output_pin.is_empty() {
                return Err(BridgeError::Parse(format!(
                    "FB call '{instance}' has empty output_pin"
                )));
            }
            format!("{instance}.{output_pin}")
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
    fn fb_call_emits_instance_decl_call_stmt_and_dot_access() {
        // The smallest possible TON example: button → 3-second delay → motor.
        // Verifies all three pieces of the FbCall transpile contract:
        //   1. `myT : TON;` lands in the internal VAR block
        //   2. `myT(IN := btn, PT := T#3s);` is emitted before the coil
        //   3. The boolean expression reads `myT.Q`
        let prog = LdProgram {
            name: "delay".into(),
            pou_type: LdPouType::Program,
            variables: vec![
                LdVariable {
                    name: "btn".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Input,
                    init: None,
                },
                LdVariable {
                    name: "motor".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Output,
                    init: None,
                },
            ],
            rungs: vec![LdRung {
                id: "delayed_start".into(),
                label: None,
                logic: LdNode::FbCall {
                    instance: "myT".into(),
                    fb_type: "TON".into(),
                    inputs: vec![
                        LdFbInput {
                            pin: "IN".into(),
                            value: LdOperand::Var { name: "btn".into() },
                        },
                        LdFbInput {
                            pin: "PT".into(),
                            value: LdOperand::Literal {
                                value: "T#3s".into(),
                            },
                        },
                    ],
                    output_pin: "Q".into(),
                },
                coils: vec![LdCoil {
                    var: "motor".into(),
                    kind: LdCoilKind::Standard,
                }],
            }],
        };
        let st = transpile_to_st(&prog).unwrap();

        // VAR declaration
        let decl_pos = st.find("myT : TON").expect("FB instance must be declared");
        // Call statement
        let call_pos = st
            .find("myT(IN := btn, PT := T#3s);")
            .expect("FB call statement must be emitted");
        // Boolean expression reads the output pin
        let use_pos = st
            .find("motor := myT.Q;")
            .expect("coil assignment must read myT.Q");

        // Declaration before use is mandatory for ironplc
        assert!(decl_pos < call_pos, "VAR decl must come before call:\n{st}");
        // Call must precede the read for IEC scan semantics
        assert!(call_pos < use_pos, "FB call must precede .Q read:\n{st}");
    }

    #[test]
    fn fb_call_inside_and_chain_renders_dot_access_in_expression() {
        // Guard pattern: only run motor while button held AND 3s elapsed.
        //   motor := btn AND myT.Q
        // The FbCall lives mid-expression and its position becomes
        // `myT.Q`; the call statement is hoisted before the assignment.
        let prog = LdProgram {
            name: "guarded".into(),
            pou_type: LdPouType::Program,
            variables: vec![
                LdVariable {
                    name: "btn".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Input,
                    init: None,
                },
                LdVariable {
                    name: "motor".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Output,
                    init: None,
                },
            ],
            rungs: vec![LdRung {
                id: "r".into(),
                label: None,
                logic: LdNode::And {
                    args: vec![
                        LdNode::Contact {
                            var: "btn".into(),
                            negated: false,
                        },
                        LdNode::FbCall {
                            instance: "myT".into(),
                            fb_type: "TON".into(),
                            inputs: vec![
                                LdFbInput {
                                    pin: "IN".into(),
                                    value: LdOperand::Var { name: "btn".into() },
                                },
                                LdFbInput {
                                    pin: "PT".into(),
                                    value: LdOperand::Literal {
                                        value: "T#3s".into(),
                                    },
                                },
                            ],
                            output_pin: "Q".into(),
                        },
                    ],
                },
                coils: vec![LdCoil {
                    var: "motor".into(),
                    kind: LdCoilKind::Standard,
                }],
            }],
        };
        let st = transpile_to_st(&prog).unwrap();
        assert!(
            st.contains("motor := (btn AND myT.Q);"),
            "expression should fold FbCall to dot-access:\n{st}"
        );
        assert!(st.contains("myT(IN := btn, PT := T#3s);"));
    }

    #[test]
    fn fb_call_dedupes_same_instance_within_rung() {
        // If a rung references the same FB instance twice (e.g. via
        // OR), only one call statement should be emitted — calling
        // edge detectors twice would mask the rising edge on the
        // second invocation.
        let prog = LdProgram {
            name: "p".into(),
            pou_type: LdPouType::Program,
            variables: vec![
                LdVariable {
                    name: "clk".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Input,
                    init: None,
                },
                LdVariable {
                    name: "out".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Output,
                    init: None,
                },
            ],
            rungs: vec![LdRung {
                id: "r".into(),
                label: None,
                logic: LdNode::Or {
                    args: vec![
                        LdNode::FbCall {
                            instance: "edge".into(),
                            fb_type: "R_TRIG".into(),
                            inputs: vec![LdFbInput {
                                pin: "CLK".into(),
                                value: LdOperand::Var { name: "clk".into() },
                            }],
                            output_pin: "Q".into(),
                        },
                        LdNode::FbCall {
                            instance: "edge".into(),
                            fb_type: "R_TRIG".into(),
                            inputs: vec![LdFbInput {
                                pin: "CLK".into(),
                                value: LdOperand::Var { name: "clk".into() },
                            }],
                            output_pin: "Q".into(),
                        },
                    ],
                },
                coils: vec![LdCoil {
                    var: "out".into(),
                    kind: LdCoilKind::Standard,
                }],
            }],
        };
        let st = transpile_to_st(&prog).unwrap();
        let n = st.matches("edge(").count();
        assert_eq!(n, 1, "FB call should be emitted exactly once:\n{st}");
    }

    #[test]
    fn fb_call_conflicting_types_for_same_instance_errors() {
        // Defensive: if a project somehow ends up with the same
        // instance name bound to two different FB types, we must
        // surface a clear error — ironplc would otherwise emit a
        // confusing duplicate-declaration error.
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
                    name: "o".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Output,
                    init: None,
                },
            ],
            rungs: vec![
                LdRung {
                    id: "r1".into(),
                    label: None,
                    logic: LdNode::FbCall {
                        instance: "t".into(),
                        fb_type: "TON".into(),
                        inputs: vec![],
                        output_pin: "Q".into(),
                    },
                    coils: vec![LdCoil {
                        var: "o".into(),
                        kind: LdCoilKind::Standard,
                    }],
                },
                LdRung {
                    id: "r2".into(),
                    label: None,
                    logic: LdNode::FbCall {
                        instance: "t".into(),
                        fb_type: "TOF".into(),
                        inputs: vec![],
                        output_pin: "Q".into(),
                    },
                    coils: vec![LdCoil {
                        var: "o".into(),
                        kind: LdCoilKind::Standard,
                    }],
                },
            ],
        };
        let err = transpile_to_st(&prog).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("conflicting types"), "got: {msg}");
    }

    #[test]
    fn fb_call_with_alternate_output_pin_uses_it_in_expression() {
        // CTUD has QU and QD; verify the renderer respects the
        // selected pin instead of always defaulting to .Q.
        let prog = LdProgram {
            name: "p".into(),
            pou_type: LdPouType::Program,
            variables: vec![
                LdVariable {
                    name: "cu".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Input,
                    init: None,
                },
                LdVariable {
                    name: "underflow".into(),
                    type_name: "BOOL".into(),
                    section: LdVarSection::Output,
                    init: None,
                },
            ],
            rungs: vec![LdRung {
                id: "r".into(),
                label: None,
                logic: LdNode::FbCall {
                    instance: "c".into(),
                    fb_type: "CTUD".into(),
                    inputs: vec![LdFbInput {
                        pin: "CU".into(),
                        value: LdOperand::Var { name: "cu".into() },
                    }],
                    output_pin: "QD".into(),
                },
                coils: vec![LdCoil {
                    var: "underflow".into(),
                    kind: LdCoilKind::Standard,
                }],
            }],
        };
        let st = transpile_to_st(&prog).unwrap();
        assert!(st.contains("c : CTUD;"));
        assert!(st.contains("underflow := c.QD;"), "got:\n{st}");
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

    // =================================================================
    //   End-to-end: our generated ST must actually compile in ironplc.
    //   These cover every standard FB the editor will expose, so that
    //   a UI regression that emits a bad pin name fails here too.
    // =================================================================

    /// Build a one-rung program with a single FbCall driving one coil.
    /// `inputs` is `(pin, var_name_or_literal)`; literals are detected
    /// by leading `T#` or digit.
    fn single_fb_rung_program(
        fb_type: &str,
        instance: &str,
        out_pin: &str,
        inputs: &[(&str, &str)],
        extra_vars: &[(&str, &str, LdVarSection)],
    ) -> LdProgram {
        let mut variables = vec![LdVariable {
            name: "out".into(),
            type_name: "BOOL".into(),
            section: LdVarSection::Output,
            init: None,
        }];
        for (name, ty, sec) in extra_vars {
            variables.push(LdVariable {
                name: (*name).into(),
                type_name: (*ty).into(),
                section: *sec,
                init: None,
            });
        }
        let fb_inputs = inputs
            .iter()
            .map(|(pin, val)| {
                let is_literal = val.starts_with("T#")
                    || val.starts_with(|c: char| c.is_ascii_digit())
                    || *val == "TRUE"
                    || *val == "FALSE";
                LdFbInput {
                    pin: (*pin).into(),
                    value: if is_literal {
                        LdOperand::Literal {
                            value: (*val).into(),
                        }
                    } else {
                        LdOperand::Var {
                            name: (*val).into(),
                        }
                    },
                }
            })
            .collect();
        LdProgram {
            name: format!("smoke_{}", instance),
            pou_type: LdPouType::Program,
            variables,
            rungs: vec![LdRung {
                id: "r".into(),
                label: None,
                logic: LdNode::FbCall {
                    instance: instance.into(),
                    fb_type: fb_type.into(),
                    inputs: fb_inputs,
                    output_pin: out_pin.into(),
                },
                coils: vec![LdCoil {
                    var: "out".into(),
                    kind: LdCoilKind::Standard,
                }],
            }],
        }
    }

    /// Asserts that transpiling `prog` yields ST that ironplc accepts
    /// without any error diagnostics. Warnings/info are tolerated.
    fn assert_compiles_clean(prog: &LdProgram) {
        let st = transpile_to_st(prog).unwrap();
        let diags = crate::check(&st);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| matches!(d.severity.as_str(), "error" | "Error"))
            .collect();
        assert!(
            errors.is_empty(),
            "ironplc rejected our ST for {}:\n--- ST ---\n{}\n--- DIAG ---\n{:#?}",
            prog.name,
            st,
            errors
        );
    }

    #[test]
    fn end_to_end_ton_compiles() {
        let prog = single_fb_rung_program(
            "TON",
            "myT",
            "Q",
            &[("IN", "btn"), ("PT", "T#3s")],
            &[("btn", "BOOL", LdVarSection::Input)],
        );
        assert_compiles_clean(&prog);
    }

    #[test]
    fn end_to_end_tof_compiles() {
        let prog = single_fb_rung_program(
            "TOF",
            "myTof",
            "Q",
            &[("IN", "trig"), ("PT", "T#500ms")],
            &[("trig", "BOOL", LdVarSection::Input)],
        );
        assert_compiles_clean(&prog);
    }

    #[test]
    fn end_to_end_tp_compiles() {
        let prog = single_fb_rung_program(
            "TP",
            "myPulse",
            "Q",
            &[("IN", "trig"), ("PT", "T#100ms")],
            &[("trig", "BOOL", LdVarSection::Input)],
        );
        assert_compiles_clean(&prog);
    }

    #[test]
    fn end_to_end_ctu_compiles() {
        let prog = single_fb_rung_program(
            "CTU",
            "myCnt",
            "Q",
            &[("CU", "click"), ("R", "rst"), ("PV", "5")],
            &[
                ("click", "BOOL", LdVarSection::Input),
                ("rst", "BOOL", LdVarSection::Input),
            ],
        );
        assert_compiles_clean(&prog);
    }

    #[test]
    fn end_to_end_ctd_compiles() {
        let prog = single_fb_rung_program(
            "CTD",
            "myDown",
            "Q",
            &[("CD", "click"), ("LD", "load"), ("PV", "5")],
            &[
                ("click", "BOOL", LdVarSection::Input),
                ("load", "BOOL", LdVarSection::Input),
            ],
        );
        assert_compiles_clean(&prog);
    }

    #[test]
    fn end_to_end_ctud_qu_compiles() {
        let prog = single_fb_rung_program(
            "CTUD",
            "myUd",
            "QU",
            &[("CU", "up"), ("CD", "dn"), ("R", "rst"), ("LD", "load"), ("PV", "5")],
            &[
                ("up", "BOOL", LdVarSection::Input),
                ("dn", "BOOL", LdVarSection::Input),
                ("rst", "BOOL", LdVarSection::Input),
                ("load", "BOOL", LdVarSection::Input),
            ],
        );
        assert_compiles_clean(&prog);
    }

    #[test]
    fn end_to_end_r_trig_compiles() {
        let prog = single_fb_rung_program(
            "R_TRIG",
            "myEdge",
            "Q",
            &[("CLK", "btn")],
            &[("btn", "BOOL", LdVarSection::Input)],
        );
        assert_compiles_clean(&prog);
    }

    #[test]
    fn end_to_end_f_trig_compiles() {
        let prog = single_fb_rung_program(
            "F_TRIG",
            "myFall",
            "Q",
            &[("CLK", "btn")],
            &[("btn", "BOOL", LdVarSection::Input)],
        );
        assert_compiles_clean(&prog);
    }

    #[test]
    fn end_to_end_sr_compiles() {
        let prog = single_fb_rung_program(
            "SR",
            "mySr",
            "Q1",
            &[("S1", "set_btn"), ("R", "rst_btn")],
            &[
                ("set_btn", "BOOL", LdVarSection::Input),
                ("rst_btn", "BOOL", LdVarSection::Input),
            ],
        );
        assert_compiles_clean(&prog);
    }

    #[test]
    fn end_to_end_rs_compiles() {
        let prog = single_fb_rung_program(
            "RS",
            "myRs",
            "Q1",
            &[("S", "set_btn"), ("R1", "rst_btn")],
            &[
                ("set_btn", "BOOL", LdVarSection::Input),
                ("rst_btn", "BOOL", LdVarSection::Input),
            ],
        );
        assert_compiles_clean(&prog);
    }
}
