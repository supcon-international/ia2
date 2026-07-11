//! `cs edge` — CRUD on deploy targets, plus the top-level `cs deploy`
//! and `cs probe` edge-orchestration commands.

use anyhow::Result;

use crate::http::{
    delete_json, get_json, http_agent, post_json, print_json, put_json, read_json_blob, url_encode,
    ServerOpt,
};
use crate::EdgeCmd;

pub(crate) fn cmd_deploy(name: &str, json: bool, server: &str) -> Result<i32> {
    // The server's /api/edges/{name}/deploy route owns the SSH+tar
    // dance — see crates/server/src/edges.rs. We just trigger it and
    // surface the report. Bigger timeout than the default agent
    // (30s) because the tar+ssh round-trip can take minutes for a
    // large project on a slow link.
    let url = format!("{server}/api/edges/{}/deploy", url_encode(name));
    let resp = http_agent()
        .post(&url)
        .timeout(std::time::Duration::from_secs(600))
        .set("Content-Type", "application/json")
        .send_json(serde_json::json!({}))
        .map_err(|e| anyhow::anyhow!("POST {url}: {e}"))?;
    let value: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        // Human-readable form: pull the version + the streamed deploy
        // log so the user sees what actually happened on the box.
        let version = value.get("version").and_then(|v| v.as_str()).unwrap_or("?");
        let log = value.get("log").and_then(|v| v.as_str()).unwrap_or("");
        if !log.is_empty() {
            eprintln!("{log}");
        }
        eprintln!("✓ deployed to '{name}' as version {version}");
    }
    // ok=false means the script ran but exited non-zero (remote failure).
    let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    Ok(if ok { 0 } else { 1 })
}

pub(crate) fn cmd_probe(name: &str, json: bool, server: &str) -> Result<i32> {
    let url = format!("{server}/api/edges/{}/probe", url_encode(name));
    let value = get_json(&url)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        let reachable = value
            .get("reachable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if reachable {
            let scans = value
                .get("scan_count")
                .and_then(|v| v.as_u64())
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".into());
            let uptime = value
                .get("uptime_secs")
                .and_then(|v| v.as_u64())
                .map(|n| format!("{n}s"))
                .unwrap_or_else(|| "?".into());
            let version = value
                .get("runtime_version")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            println!("✓ {name} reachable · v{version} · {scans} scans · up {uptime}");
        } else {
            let err = value
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unreachable");
            eprintln!("✗ {name}: {err}");
        }
    }
    let reachable = value
        .get("reachable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Ok(if reachable { 0 } else { 1 })
}

pub(crate) fn cmd_edge(cmd: EdgeCmd) -> Result<i32> {
    match cmd {
        EdgeCmd::Create {
            name,
            host,
            server: ServerOpt { server },
        } => {
            let resp = post_json(
                &format!("{server}/api/edges"),
                &serde_json::json!({ "name": name, "host": host }),
            )?;
            print_json(&resp)
        }
        EdgeCmd::List {
            json,
            server: ServerOpt { server },
        } => {
            let tree = get_json(&format!("{server}/api/project"))?;
            let edges = tree.get("edges").cloned().unwrap_or(serde_json::json!([]));
            if json {
                println!("{}", serde_json::to_string_pretty(&edges)?);
            } else if let Some(arr) = edges.as_array() {
                if arr.is_empty() {
                    eprintln!("no edges");
                } else {
                    for e in arr {
                        let n = e.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let h = e.get("host").and_then(|v| v.as_str()).unwrap_or("?");
                        println!("{n:<24}  {h}");
                    }
                }
            }
            Ok(0)
        }
        EdgeCmd::Get {
            name,
            server: ServerOpt { server },
        } => {
            let resp = get_json(&format!("{server}/api/edges/{}", url_encode(&name)))?;
            print_json(&resp)
        }
        EdgeCmd::Set {
            name,
            from,
            server: ServerOpt { server },
        } => {
            let body = read_json_blob(&from)?;
            let resp = put_json(&format!("{server}/api/edges/{}", url_encode(&name)), &body)?;
            print_json(&resp)
        }
        EdgeCmd::Delete {
            name,
            server: ServerOpt { server },
        } => {
            let resp = delete_json(&format!("{server}/api/edges/{}", url_encode(&name)))?;
            print_json(&resp)
        }
        EdgeCmd::Logs {
            name,
            tail,
            server: ServerOpt { server },
        } => {
            let url = format!("{server}/api/edges/{}/logs?tail={tail}", url_encode(&name));
            let resp = get_json(&url)?;
            if let Some(lines) = resp.get("lines").and_then(|v| v.as_array()) {
                for line in lines {
                    if let Some(s) = line.as_str() {
                        println!("{s}");
                    }
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            }
            Ok(0)
        }
        EdgeCmd::Scan {
            name,
            json,
            server: ServerOpt { server },
        } => {
            let url = format!("{server}/api/edges/{}/discover", url_encode(&name));
            let resp = get_json(&url)?;
            if json {
                return print_json(&resp);
            }
            let Some(devs) = resp.as_array() else {
                return print_json(&resp);
            };
            if devs.is_empty() {
                eprintln!("no devices in project");
            }
            for d in devs {
                let dname = d.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let proto = d.get("protocol").and_then(|v| v.as_str()).unwrap_or("?");
                let connected = d
                    .get("connected")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !connected {
                    let err = d
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("not connected");
                    println!("✗ {dname} ({proto}) — {err}");
                    continue;
                }
                let slaves = d.get("slaves").and_then(|v| v.as_array());
                let n = slaves.map(|a| a.len()).unwrap_or(0);
                println!("✓ {dname} ({proto}) connected · {n} slave(s)");
                if let Some(arr) = slaves {
                    for s in arr {
                        let idx = s.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                        let sn = s.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let vid = s.get("vendor_id").and_then(|v| v.as_u64()).unwrap_or(0);
                        let pid = s.get("product_id").and_then(|v| v.as_u64()).unwrap_or(0);
                        let inb = s.get("input_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
                        let outb = s.get("output_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
                        println!(
                            "    [{idx}] {sn}  vendor=0x{vid:08x} product=0x{pid:08x}  in={inb}B out={outb}B"
                        );
                    }
                }
            }
            Ok(0)
        }
        EdgeCmd::System {
            name,
            json,
            server: ServerOpt { server },
        } => {
            let url = format!("{server}/api/edges/{}/system", url_encode(&name));
            let resp = get_json(&url)?;
            if json {
                return print_json(&resp);
            }
            let arch = resp.get("arch").and_then(|v| v.as_str()).unwrap_or("?");
            let os = resp.get("os").and_then(|v| v.as_str()).unwrap_or("?");
            println!("{os}/{arch}");
            if let Some(nics) = resp.get("nics").and_then(|v| v.as_array()) {
                println!("NICs:");
                for n in nics {
                    let nm = n.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let st = n.get("operstate").and_then(|v| v.as_str()).unwrap_or("?");
                    let carrier = n.get("carrier").and_then(|v| v.as_bool()).unwrap_or(false);
                    let mac = n.get("mac").and_then(|v| v.as_str()).unwrap_or("");
                    let link = if carrier { "carrier" } else { "no-carrier" };
                    println!("  {nm:<16} {st:<8} {link:<11} {mac}");
                }
            }
            match resp.get("serial_ports").and_then(|v| v.as_array()) {
                Some(ports) if !ports.is_empty() => {
                    println!("serial ports:");
                    for p in ports {
                        if let Some(s) = p.as_str() {
                            println!("  {s}");
                        }
                    }
                }
                _ => println!("serial ports: (none)"),
            }
            Ok(0)
        }
    }
}
