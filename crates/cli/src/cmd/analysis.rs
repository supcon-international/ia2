//! Offline, bridge-direct subcommands — `check`, `transpile`, `explain`,
//! `symbols`. These never touch the HTTP server; they call the ironplc
//! bridge and the project store directly.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use ironplc_bridge::CheckDiagnostic;
use project::PouLanguage;

pub(crate) fn cmd_check(files: &[PathBuf], json: bool, explain: bool) -> Result<i32> {
    // The files are checked TOGETHER: each one is analysed with the
    // others as declaration context, so `cs check pous/*.st` resolves
    // FUNCTION_BLOCKs declared in sibling files exactly like a project
    // compile would. A single file behaves as before (empty context).
    let mut inputs = Vec::with_capacity(files.len());
    for file in files {
        let language = language_for_path(file)?;
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        inputs.push((file.display().to_string(), source, language));
    }
    let per_file = ironplc_bridge::check_sources_together(&inputs);

    let mut all: Vec<FileDiagnostics> = Vec::with_capacity(files.len());
    let mut any_errors = false;
    for (file, diags) in files.iter().zip(per_file) {
        if !diags.is_empty() {
            any_errors = true;
        }
        all.push(FileDiagnostics {
            file: file.clone(),
            diagnostics: diags,
        });
    }

    if json {
        let value: serde_json::Value = serde_json::json!({
            "ok": !any_errors,
            "files": all.iter().map(|f| serde_json::json!({
                "file": f.file.to_string_lossy(),
                "diagnostics": &f.diagnostics,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        for f in &all {
            print_diagnostics_human(&f.file, &f.diagnostics, explain);
        }
        let total: usize = all.iter().map(|f| f.diagnostics.len()).sum();
        if total == 0 {
            eprintln!(
                "✓ {} file{} clean",
                files.len(),
                if files.len() == 1 { "" } else { "s" }
            );
        } else {
            eprintln!(
                "✗ {} error{} across {} file{}",
                total,
                if total == 1 { "" } else { "s" },
                all.iter().filter(|f| !f.diagnostics.is_empty()).count(),
                if files.len() == 1 { "" } else { "s" },
            );
        }
    }

    Ok(if any_errors { 1 } else { 0 })
}

struct FileDiagnostics {
    file: PathBuf,
    diagnostics: Vec<CheckDiagnostic>,
}

fn print_diagnostics_human(file: &Path, diags: &[CheckDiagnostic], explain: bool) {
    if diags.is_empty() {
        return;
    }
    let f = file.display();
    for d in diags {
        // Exactly one of ld / fbd / sfc location is populated for
        // graphical POUs; all are None for ST. Order doesn't matter —
        // they're mutually exclusive by construction.
        let loc_hint = if let Some(loc) = &d.ld_location {
            format!(" [{}]", describe_ld_location(loc))
        } else if let Some(loc) = &d.fbd_location {
            format!(" [{}]", describe_fbd_location(loc))
        } else if let Some(loc) = &d.sfc_location {
            format!(" [{}]", describe_sfc_location(loc))
        } else {
            String::new()
        };
        eprintln!(
            "{f}:{}:{}: {} {}{loc_hint}: {}",
            d.start_line, d.start_column, d.severity, d.code, d.message,
        );
        // Context lines under the primary message, indented. These
        // are ironplc's `described` entries — almost always one short
        // structured fragment like `variable=foo` or `type=BOOL`.
        for c in &d.context {
            eprintln!("    {c}");
        }
        // Related labels — point at secondary locations like "did you
        // mean: bar?" or "first declared here". We print them as
        // file:line:col-prefixed notes so they're parseable by the
        // same regex an editor would use to jump.
        for r in &d.related {
            eprintln!(
                "    note: {f}:{}:{}: {}",
                r.start_line, r.start_column, r.message,
            );
        }
        // Full explanation when `--explain` is set. Indent every
        // line by two spaces so the prose is visually nested under
        // the diagnostic rather than competing with it.
        if explain {
            if let Some(expl) = &d.explanation {
                eprintln!();
                for line in expl.lines() {
                    eprintln!("  {line}");
                }
                eprintln!();
            }
        }
    }
}

fn describe_ld_location(loc: &ironplc_bridge::LdLocation) -> String {
    use ironplc_bridge::LdLocation::*;
    match loc {
        Variable { name } => format!("var {name}"),
        Rung { rung_id } => format!("rung {rung_id}"),
        Coil {
            rung_id,
            coil_index,
        } => format!("rung {rung_id} · coil {coil_index}"),
        FbCall { rung_id, instance } => format!("rung {rung_id} · {instance}(…)"),
    }
}

fn describe_fbd_location(loc: &ironplc_bridge::FbdLocation) -> String {
    use ironplc_bridge::FbdLocation::*;
    match loc {
        Variable { name } => format!("var {name}"),
        Block { block_id } => format!("block {block_id}"),
        Output { variable } => format!("output {variable}"),
    }
}

fn describe_sfc_location(loc: &ironplc_bridge::SfcLocation) -> String {
    use ironplc_bridge::SfcLocation::*;
    match loc {
        Variable { name } => format!("var {name}"),
        Step { name } => format!("step {name}"),
        Action { step, action_index } => format!("step {step} · action {action_index}"),
        Transition { index } => format!("transition #{index}"),
    }
}

pub(crate) fn cmd_transpile(file: &Path, with_map: bool) -> Result<i32> {
    let language = language_for_path(file)?;
    let source =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;

    match language {
        PouLanguage::St => {
            // ST is its own intermediate; nothing to do. Echo it so the
            // command remains useful in pipelines that don't care about
            // language at the caller's side.
            if with_map {
                eprintln!("note: --with-map has no effect for ST sources");
            }
            print!("{source}");
            Ok(0)
        }
        PouLanguage::Ld => {
            let prog: project::LdProgram = serde_json::from_str(&source)
                .with_context(|| format!("parsing LD JSON in {}", file.display()))?;
            let (st, map) = ironplc_bridge::transpile_ld_to_st_with_map(&prog)
                .with_context(|| format!("transpiling {}", file.display()))?;
            if with_map {
                // Serialise the map alongside the ST — JSON output, one
                // pair per call. The map.lines field is a Vec<Option<…>>
                // which serde renders as `[null, {…}, null, …]`.
                let payload = serde_json::json!({
                    "st": st,
                    "source_map": map.lines,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{st}");
            }
            Ok(0)
        }
        PouLanguage::Fbd => {
            let prog: project::FbdProgram = serde_json::from_str(&source)
                .with_context(|| format!("parsing FBD JSON in {}", file.display()))?;
            let (st, map) = ironplc_bridge::transpile_fbd_to_st_with_map(&prog)
                .with_context(|| format!("transpiling {}", file.display()))?;
            if with_map {
                let payload = serde_json::json!({
                    "st": st,
                    "source_map": map.lines,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{st}");
            }
            Ok(0)
        }
        PouLanguage::Sfc => {
            let prog: project::SfcProgram = serde_json::from_str(&source)
                .with_context(|| format!("parsing SFC JSON in {}", file.display()))?;
            let (st, map) = ironplc_bridge::transpile_sfc_to_st_with_map(&prog)
                .with_context(|| format!("transpiling {}", file.display()))?;
            if with_map {
                let payload = serde_json::json!({
                    "st": st,
                    "source_map": map.lines,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{st}");
            }
            Ok(0)
        }
    }
}

pub(crate) fn cmd_explain(code: &str) -> Result<i32> {
    match ironplc_bridge::lookup_problem_doc(code) {
        Some((rst, title)) => {
            // Print the title line first so a quick `cs explain P4007`
            // tells you what the code is for without scanning the body.
            // The full RST follows verbatim — agents and humans can
            // both read it. (rST format is text-friendly so we don't
            // try to render it.)
            println!("{code} — {title}");
            println!();
            print!("{rst}");
            Ok(0)
        }
        None => {
            eprintln!("error: no documentation for `{code}` — not in ironplc's problem registry");
            Ok(1)
        }
    }
}

pub(crate) fn cmd_symbols(file: &Path, name_filter: Option<&str>, json: bool) -> Result<i32> {
    let language = language_for_path(file)?;
    let source =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let mut syms = ironplc_bridge::extract_symbols(&source, language);
    if let Some(needle) = name_filter {
        syms.retain(|s| s.name.contains(needle));
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&syms)?);
    } else {
        // Tabular: aligned `direction  name : type_name`. Direction
        // pads to the widest width so columns line up.
        let pad = syms.iter().map(|s| s.direction.len()).max().unwrap_or(0);
        for s in &syms {
            println!(
                "{:<pad$}  {} : {}",
                s.direction,
                s.name,
                s.type_name,
                pad = pad,
            );
        }
        eprintln!(
            "{} symbol{}",
            syms.len(),
            if syms.len() == 1 { "" } else { "s" },
        );
    }
    Ok(if syms.is_empty() && name_filter.is_some() {
        1
    } else {
        0
    })
}

/// Map a file path to its POU language by extension. `.ld.json` is the
/// canonical LD extension (see MEMORY/graphical-languages.md); plain
/// `.st` is ST. Anything else is an error rather than a silent default
/// — agents should know which path they're on.
fn language_for_path(path: &Path) -> Result<PouLanguage> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .with_context(|| format!("invalid filename: {}", path.display()))?;
    // Order: longest known suffix first (`.ld.json` must beat `.st`'s
    // would-be ".json" eyeball check; `.fbd.json` and `.sfc.json` must
    // not collide with a generic `.json`).
    if name.ends_with(".ld.json") {
        Ok(PouLanguage::Ld)
    } else if name.ends_with(".fbd.json") {
        Ok(PouLanguage::Fbd)
    } else if name.ends_with(".sfc.json") {
        Ok(PouLanguage::Sfc)
    } else if name.ends_with(".st") {
        Ok(PouLanguage::St)
    } else {
        bail!(
            "can't infer language from filename {name:?} — expected .st, .ld.json, .fbd.json, or .sfc.json"
        )
    }
}
