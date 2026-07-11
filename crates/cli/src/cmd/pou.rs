//! `cs pou` — create / save / delete POU files in the open project.

use anyhow::{Context, Result};

use crate::http::{delete_json, http_agent, post_json, print_json, url_encode, ServerOpt};
use crate::PouCmd;

pub(crate) fn cmd_pou(cmd: PouCmd) -> Result<i32> {
    match cmd {
        PouCmd::Create {
            path,
            language,
            r#type,
            server: ServerOpt { server },
        } => {
            let resp = post_json(
                &format!("{server}/api/pous"),
                // Server's CreatePouRequest uses `type` (renamed
                // from Rust `type_` via serde). Language values match
                // the on-disk extensions: st / ld / fbd / sfc.
                &serde_json::json!({
                    "path": path,
                    "type": r#type,
                    "language": language,
                }),
            )?;
            print_json(&resp)
        }
        PouCmd::Save {
            path,
            from,
            stdin,
            server: ServerOpt { server },
        } => {
            let source = if let Some(file) = from {
                std::fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))?
            } else {
                // Read stdin (whether `--stdin` is set or it's the
                // implicit default).
                let _ = stdin;
                let mut s = String::new();
                use std::io::Read;
                std::io::stdin()
                    .read_to_string(&mut s)
                    .context("reading source from stdin")?;
                s
            };
            // `save_pou` accepts text/plain, not JSON — wire format
            // matches the IDE editor's auto-save path.
            let url = format!("{server}/api/pous/{}", url_encode(&path));
            let resp = http_agent()
                .put(&url)
                .set("Content-Type", "text/plain")
                .send_string(&source)
                .map_err(|e| anyhow::anyhow!("PUT {url}: {e}"))?;
            let value: serde_json::Value = resp
                .into_json()
                .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
            print_json(&value)
        }
        PouCmd::Delete {
            path,
            server: ServerOpt { server },
        } => {
            let resp = delete_json(&format!("{server}/api/pous/{}", url_encode(&path)))?;
            print_json(&resp)
        }
    }
}
