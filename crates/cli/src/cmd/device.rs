//! `cs device` — CRUD on devices in the open project.

use anyhow::Result;

use crate::http::{
    delete_json, get_json, post_json, print_json, put_json, read_json_blob, url_encode, ServerOpt,
};
use crate::DeviceCmd;

pub(crate) fn cmd_device(cmd: DeviceCmd) -> Result<i32> {
    match cmd {
        DeviceCmd::Create {
            name,
            protocol,
            server: ServerOpt { server },
        } => {
            let resp = post_json(
                &format!("{server}/api/devices"),
                &serde_json::json!({ "name": name, "protocol": protocol }),
            )?;
            print_json(&resp)
        }
        DeviceCmd::List {
            json,
            server: ServerOpt { server },
        } => {
            // Devices live inside ProjectTree — call /api/project and
            // pluck the `devices` array. Cheap enough; avoids a new
            // dedicated endpoint for what's already exposed.
            let tree = get_json(&format!("{server}/api/project"))?;
            let devices = tree
                .get("devices")
                .cloned()
                .unwrap_or(serde_json::json!([]));
            if json {
                println!("{}", serde_json::to_string_pretty(&devices)?);
            } else if let Some(arr) = devices.as_array() {
                if arr.is_empty() {
                    eprintln!("no devices");
                } else {
                    for d in arr {
                        let n = d.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let p = d.get("protocol").and_then(|v| v.as_str()).unwrap_or("?");
                        println!("{p:<10}  {n}");
                    }
                }
            }
            Ok(0)
        }
        DeviceCmd::Get {
            name,
            server: ServerOpt { server },
        } => {
            let resp = get_json(&format!("{server}/api/devices/{}", url_encode(&name)))?;
            print_json(&resp)
        }
        DeviceCmd::Set {
            name,
            from,
            server: ServerOpt { server },
        } => {
            let body = read_json_blob(&from)?;
            let resp = put_json(
                &format!("{server}/api/devices/{}", url_encode(&name)),
                &body,
            )?;
            print_json(&resp)
        }
        DeviceCmd::Delete {
            name,
            server: ServerOpt { server },
        } => {
            let resp = delete_json(&format!("{server}/api/devices/{}", url_encode(&name)))?;
            print_json(&resp)
        }
        DeviceCmd::EsiAssemble {
            name,
            idents,
            server: ServerOpt { server },
        } => {
            let detected = idents
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(parse_module_ident)
                .collect::<Result<Vec<u32>>>()?;
            let body = serde_json::json!({ "detected": detected });
            let resp = post_json(
                &format!("{server}/api/devices/{}/esi-assemble", url_encode(&name)),
                &body,
            )?;
            // Summarize the assembled channels. The Device JSON is flat —
            // protocol fields (including `channels`) sit at the top level.
            let n = resp
                .get("channels")
                .and_then(|c| c.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            println!(
                "✓ assembled {n} channels from ESI for '{name}' ({} modules)",
                detected.len()
            );
            print_json(&resp)
        }
    }
}

/// Parse a module ident in `0x..` hex or decimal form.
fn parse_module_ident(s: &str) -> Result<u32> {
    let t = s.trim();
    let parsed = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(h, 16)
    } else {
        t.parse::<u32>()
    };
    parsed.map_err(|e| anyhow::anyhow!("bad module ident {s:?}: {e}"))
}
