//! Runtime lifecycle (`cs run` / `cs stop`) and the debug-control trio
//! (`cs runtime pause/resume/step/status/force/unforce/write`), plus the
//! value-encoding helpers the force/write paths use.

use std::path::Path;

use anyhow::{Context, Result};

use crate::http::{delete_json, get_json, post_json, print_json, url_encode, ServerOpt};
use crate::RuntimeCmd;

pub(crate) fn cmd_run(program: Option<&str>, file: Option<&Path>, server: &str) -> Result<i32> {
    // The server distinguishes three run shapes by the presence of
    // `program` / `file_path`. Mirror that here.
    let body = match (program, file) {
        (None, None) => serde_json::json!({ "kind": "project" }),
        (Some(name), None) => serde_json::json!({
            "kind": "isolated",
            "program": name,
        }),
        (Some(name), Some(path)) => {
            let abs = path
                .canonicalize()
                .with_context(|| format!("resolving {}", path.display()))?;
            serde_json::json!({
                "kind": "isolated",
                "program": name,
                "file_path": abs.display().to_string(),
            })
        }
        (None, Some(_)) => {
            anyhow::bail!("--file requires --program to name the PROGRAM inside it")
        }
    };
    let resp = post_json(&format!("{server}/api/run"), &body)?;
    print_json(&resp)
}

pub(crate) fn cmd_stop(server: &str) -> Result<i32> {
    let resp = post_json(&format!("{server}/api/stop"), &())?;
    print_json(&resp)
}

pub(crate) fn cmd_runtime(cmd: RuntimeCmd) -> Result<i32> {
    match cmd {
        RuntimeCmd::Pause {
            edge,
            server: ServerOpt { server },
        } => {
            let resp = match &edge {
                Some(e) => post_json(
                    &format!("{server}/api/edges/{}/runtime/pause", url_encode(e)),
                    &serde_json::json!({}),
                )?,
                None => post_json(&format!("{server}/api/runtime/pause"), &())?,
            };
            print_json(&resp)
        }
        RuntimeCmd::Resume {
            edge,
            server: ServerOpt { server },
        } => {
            let resp = match &edge {
                Some(e) => post_json(
                    &format!("{server}/api/edges/{}/runtime/resume", url_encode(e)),
                    &serde_json::json!({}),
                )?,
                None => post_json(&format!("{server}/api/runtime/resume"), &())?,
            };
            print_json(&resp)
        }
        RuntimeCmd::Step {
            cycles,
            edge,
            server: ServerOpt { server },
        } => {
            let body = serde_json::json!({ "cycles": cycles });
            let resp = match &edge {
                Some(e) => post_json(
                    &format!("{server}/api/edges/{}/runtime/step", url_encode(e)),
                    &body,
                )?,
                None => post_json(&format!("{server}/api/runtime/step"), &body)?,
            };
            print_json(&resp)
        }
        RuntimeCmd::Status {
            json,
            edge,
            server: ServerOpt { server },
        } => {
            // Local: /api/runtime/status. Edge: the runtime's /status via
            // the server proxy (different shape, but carries mode + forces).
            let status = match &edge {
                Some(e) => get_json(&format!("{server}/api/edges/{}/status", url_encode(e)))?,
                None => get_json(&format!("{server}/api/runtime/status"))?,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                // A minimal human summary; full status is one --json
                // away.
                let mode = status
                    .get("mode")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let forces = status
                    .get("forces")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                // Edge /status has no `running` bool — derive from mode.
                let running = status
                    .get("running")
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| {
                        mode.get("kind").and_then(|k| k.as_str()) == Some("running")
                    });
                println!(
                    "running: {running}  mode: {}  forces: {}",
                    serde_json::to_string(&mode)?,
                    forces.len(),
                );
                for f in &forces {
                    if let (Some(n), Some(v)) =
                        (f.get("name").and_then(|v| v.as_str()), f.get("value"))
                    {
                        println!("  {n} := {v}");
                    }
                }
            }
            Ok(0)
        }
        RuntimeCmd::Force {
            name,
            value,
            edge,
            server: ServerOpt { server },
        } => {
            let resp = match &edge {
                Some(e) => {
                    let encoded =
                        pack_value(&name, edge_var_type(&server, e, &name).as_deref(), &value)?;
                    post_json(
                        &format!("{server}/api/edges/{}/runtime/force", url_encode(e)),
                        &serde_json::json!({ "name": name, "value": encoded }),
                    )?
                }
                None => {
                    let encoded = parse_value(&server, &name, &value)?;
                    post_json(
                        &format!("{server}/api/runtime/forces/{}", url_encode(&name)),
                        &serde_json::json!({ "value": encoded }),
                    )?
                }
            };
            print_json(&resp)
        }
        RuntimeCmd::Unforce {
            name,
            edge,
            server: ServerOpt { server },
        } => {
            let resp = match &edge {
                Some(e) => post_json(
                    &format!("{server}/api/edges/{}/runtime/unforce", url_encode(e)),
                    &serde_json::json!({ "name": name }),
                )?,
                None => delete_json(&format!(
                    "{server}/api/runtime/forces/{}",
                    url_encode(&name)
                ))?,
            };
            print_json(&resp)
        }
        RuntimeCmd::Write {
            name,
            value,
            edge,
            server: ServerOpt { server },
        } => {
            let resp = match &edge {
                Some(e) => {
                    let encoded =
                        pack_value(&name, edge_var_type(&server, e, &name).as_deref(), &value)?;
                    post_json(
                        &format!("{server}/api/edges/{}/runtime/write", url_encode(e)),
                        &serde_json::json!({ "name": name, "value": encoded }),
                    )?
                }
                None => {
                    let encoded = parse_value(&server, &name, &value)?;
                    post_json(
                        &format!("{server}/api/runtime/variables/{}", url_encode(&name)),
                        &serde_json::json!({ "value": encoded }),
                    )?
                }
            };
            print_json(&resp)
        }
    }
}

/// Convert a human-typed value into the i32 the runtime wire protocol
/// expects, type-aware via the runtime's snapshot.
///
/// Why: the bridge stores all variables — BOOL, INT, REAL, … — in
/// 32-bit slots and the force/write endpoint takes a raw `i32`. For
/// REAL the i32 is the IEEE-754 bit pattern of the float, NOT the
/// integer value. Without type info, `cs runtime force x 50.0` would
/// have to send `1112014848`. This helper does the conversion so
/// humans (and agents) can use natural notation.
///
/// Strategy:
///   1. If the value is obviously BOOL ("true"/"false" case-insensitive)
///      → 0 / 1.
///   2. Otherwise fetch `/api/runtime/snapshot`, look up the variable,
///      encode based on its `type_name` (REAL → bit-pack, INT-family
///      → as-is).
///   3. If the snapshot doesn't include the variable (runtime not
///      running yet, or the variable lives in a POU instance the
///      bridge's snapshot extractor doesn't traverse — a known bridge
///      bug as of 2026-05), fall back to format-based sniffing: a
///      decimal point implies REAL, otherwise INT. Print a stderr
///      note so users know we guessed.
fn parse_value(server: &str, name: &str, raw: &str) -> Result<i32> {
    let var_type = snapshot_var_type(server, name).unwrap_or_default();
    pack_value(name, var_type.as_deref(), raw)
}

/// Resolve an edge variable's type from the edge runtime's `/status`
/// (last snapshot, which carries per-variable `type_name`).
fn edge_var_type(server: &str, edge: &str, name: &str) -> Option<String> {
    let status = get_json(&format!("{server}/api/edges/{}/status", url_encode(edge))).ok()?;
    let vars = status.get("last_snapshot")?.get("vars")?.as_array()?;
    for v in vars {
        if v.get("name").and_then(|n| n.as_str()) == Some(name) {
            return v
                .get("type_name")
                .and_then(|t| t.as_str())
                .map(String::from);
        }
    }
    None
}

/// Bit-pack a human value string into the i32 force/write wire, given the
/// variable's IEC `var_type` (None = unknown → guess from value format).
fn pack_value(name: &str, var_type: Option<&str>, raw: &str) -> Result<i32> {
    // BOOL shortcuts. Case-insensitive because TRUE/FALSE are the IEC
    // canonical form but agents type either.
    match raw.to_ascii_lowercase().as_str() {
        "true" => return Ok(1),
        "false" => return Ok(0),
        _ => {}
    }

    match var_type {
        Some("BOOL") => {
            // We already handled TRUE/FALSE above; accept 0/1 too.
            let n: i32 = raw.parse().with_context(|| {
                format!("value `{raw}` doesn't fit BOOL (expected TRUE/FALSE/1/0)")
            })?;
            Ok(if n != 0 { 1 } else { 0 })
        }
        Some("REAL") => {
            let f: f32 = raw
                .parse()
                .with_context(|| format!("value `{raw}` doesn't parse as REAL (32-bit float)"))?;
            Ok(f.to_bits() as i32)
        }
        Some("LREAL") => {
            anyhow::bail!(
                "LREAL (64-bit float) doesn't fit the 32-bit force wire — \
                 use a REAL variable, or write the low 32 bits manually"
            )
        }
        Some(int_type)
            if matches!(
                int_type,
                "INT" | "DINT" | "SINT" | "UINT" | "UDINT" | "USINT" | "BYTE" | "WORD" | "DWORD"
            ) =>
        {
            let n: i64 = raw.parse().with_context(|| {
                format!("value `{raw}` doesn't parse as integer for {int_type}")
            })?;
            // Wire is i32; for unsigned and larger types we just bit-
            // truncate. Users wanting precise unsigned semantics can
            // pass the i32 reinterpretation directly.
            Ok(n as i32)
        }
        Some(other) => {
            anyhow::bail!("don't know how to encode value `{raw}` for type {other} (yet)")
        }
        None => {
            // No type info — guess from format and warn loudly.
            if raw.contains('.') || raw.contains('e') || raw.contains('E') {
                let f: f32 = raw.parse().with_context(|| {
                    format!("value `{raw}` looks like a float but doesn't parse as f32")
                })?;
                eprintln!(
                    "note: runtime didn't expose `{name}`'s type — guessed REAL from value format"
                );
                Ok(f.to_bits() as i32)
            } else {
                let n: i32 = raw.parse().with_context(|| {
                    format!("value `{raw}` doesn't parse as i32; if you meant REAL, use `{raw}.0`")
                })?;
                eprintln!("note: runtime didn't expose `{name}`'s type — assumed INT family");
                Ok(n)
            }
        }
    }
}

/// Best-effort variable type lookup via `/api/runtime/snapshot`. The
/// snapshot returns one record per live variable with `type_name`. If
/// the runtime isn't running, or the bridge's extractor doesn't
/// include this variable's POU, return Ok(None) and let the caller
/// fall back to format-sniffing.
fn snapshot_var_type(server: &str, name: &str) -> Result<Option<String>> {
    let snap = match get_json(&format!("{server}/api/runtime/snapshot")) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let vars = match snap.get("vars").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Ok(None),
    };
    for v in vars {
        if v.get("name").and_then(|n| n.as_str()) == Some(name) {
            return Ok(v
                .get("type_name")
                .and_then(|t| t.as_str())
                .map(String::from));
        }
    }
    Ok(None)
}
