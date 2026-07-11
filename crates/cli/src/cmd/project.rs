//! `cs project` — validate / inspect a project directory, plus the
//! open-project lifecycle (create / open / close / list) over HTTP.

use std::path::Path;

use anyhow::{Context, Result};
use project::ProjectStore;

use crate::http::{get_json, post_json, print_json};

pub(crate) fn cmd_project_check(path: &Path, json: bool) -> Result<i32> {
    let store = open_project(path)?;
    let outcome = ironplc_bridge::compile_project(&store);
    let (ok, message): (bool, String) = match outcome {
        Ok(_) => (true, "clean".into()),
        Err(e) => (false, format!("{e:?}")),
    };

    if json {
        let value = serde_json::json!({
            "ok": ok,
            "project": store.name(),
            "message": message,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if ok {
        eprintln!("✓ project {} compiles cleanly", store.name());
    } else {
        eprintln!("✗ project {} failed to compile:", store.name());
        eprintln!("{message}");
    }

    Ok(if ok { 0 } else { 1 })
}

pub(crate) fn cmd_project_info(path: &Path, json: bool) -> Result<i32> {
    let store = open_project(path)?;
    let pous = store
        .list_pou_paths()
        .with_context(|| "listing POU files")?;
    let devices = store.list_devices().with_context(|| "listing devices")?;
    let edges = store.list_edges().with_context(|| "listing edges")?;

    if json {
        let value = serde_json::json!({
            "name": store.name(),
            "root": store.root().display().to_string(),
            "pous": pous,
            "devices": devices.iter().map(|d| serde_json::json!({
                "name": &d.name,
                "protocol": format!("{:?}", d.config.protocol()),
            })).collect::<Vec<_>>(),
            "edges": edges.iter().map(|e| serde_json::json!({
                "name": &e.name,
                "host": &e.host,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("project: {}", store.name());
        println!("root:    {}", store.root().display());
        println!();
        println!("POUs ({}):", pous.len());
        for p in &pous {
            println!("  {p}");
        }
        println!();
        println!("Devices ({}):", devices.len());
        for d in &devices {
            println!("  {} ({:?})", d.name, d.config.protocol());
        }
        println!();
        println!("Edges ({}):", edges.len());
        for e in &edges {
            println!("  {} → {}", e.name, e.host);
        }
    }

    Ok(0)
}

// Wrap the HTTP API so agents call `cs project create foo` instead
// of `curl -X POST localhost:3001/api/projects -d '{"name":"foo"}'`.
// Symmetric with `cs project info / check` which already operate on
// project directories.

pub(crate) fn cmd_project_create(name: &str, server: &str) -> Result<i32> {
    let resp = post_json(
        &format!("{server}/api/projects"),
        &serde_json::json!({ "name": name }),
    )?;
    print_json(&resp)
}

pub(crate) fn cmd_project_open(path: &Path, server: &str) -> Result<i32> {
    let abs = path
        .canonicalize()
        .with_context(|| format!("resolving {}", path.display()))?;
    let resp = post_json(
        &format!("{server}/api/projects/open"),
        &serde_json::json!({ "path": abs.display().to_string() }),
    )?;
    print_json(&resp)
}

pub(crate) fn cmd_project_close(server: &str) -> Result<i32> {
    let resp = post_json(&format!("{server}/api/projects/close"), &())?;
    print_json(&resp)
}

pub(crate) fn cmd_project_list(server: &str, json: bool) -> Result<i32> {
    let value = get_json(&format!("{server}/api/projects/open-list"))?;
    if json {
        return print_json(&value);
    }
    // Human-readable: active marked with `*`, names padded into a
    // column. Path on the right for orientation.
    let active = value
        .get("active")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let projects = value
        .get("projects")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if projects.is_empty() {
        eprintln!("no projects open");
        return Ok(0);
    }
    let name_width = projects
        .iter()
        .filter_map(|p| p.get("name").and_then(|v| v.as_str()).map(str::len))
        .max()
        .unwrap_or(0);
    for p in &projects {
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let path = p.get("path").and_then(|v| v.as_str()).unwrap_or("?");
        let marker = if name == active { "*" } else { " " };
        println!("{marker} {name:<name_width$}  {path}");
    }
    eprintln!(
        "{} project{} open · active marked with *",
        projects.len(),
        if projects.len() == 1 { "" } else { "s" },
    );
    Ok(0)
}

/// Open a project store at `path`. Resolves `.` to the current working
/// directory so `cs project check` (no args) does the right thing.
fn open_project(path: &Path) -> Result<ProjectStore> {
    let abs = if path.as_os_str() == "." {
        std::env::current_dir().context("resolving current directory")?
    } else {
        path.to_path_buf()
    };
    ProjectStore::open(abs.clone()).with_context(|| format!("opening project at {}", abs.display()))
}
