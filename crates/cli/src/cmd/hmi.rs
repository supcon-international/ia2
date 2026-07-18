//! `cs hmi` — operator-screen authoring. Thin JSON passthroughs over the
//! /api/hmi family; the interesting part is the workflow the help text
//! teaches (generate a baseline, then reshape element-by-element with
//! `op`, watching the IDE canvas render each batch live).

use anyhow::Result;

use crate::http::{
    delete_json, get_json, post_json, print_json, put_json, read_json_blob, url_encode, ServerOpt,
};
use crate::HmiCmd;

pub(crate) fn cmd_hmi(cmd: HmiCmd) -> Result<i32> {
    match cmd {
        HmiCmd::List {
            server: ServerOpt { server },
        } => {
            let resp = get_json(&format!("{server}/api/hmi"))?;
            print_json(&resp)
        }
        HmiCmd::Get {
            path,
            server: ServerOpt { server },
        } => {
            let resp = get_json(&format!("{server}/api/hmi/{}", url_encode(&path)))?;
            print_json(&resp)
        }
        HmiCmd::Create {
            path,
            title,
            server: ServerOpt { server },
        } => {
            let body = serde_json::json!({ "path": path, "title": title });
            let resp = post_json(&format!("{server}/api/hmi"), &body)?;
            print_json(&resp)
        }
        HmiCmd::Save {
            path,
            from,
            server: ServerOpt { server },
        } => {
            let body = read_json_blob(&from)?;
            let resp = put_json(&format!("{server}/api/hmi/{}", url_encode(&path)), &body)?;
            print_json(&resp)
        }
        HmiCmd::Op {
            path,
            from,
            server: ServerOpt { server },
        } => {
            let raw = read_json_blob(&from)?;
            // Accept both `{"ops":[...]}` and a bare `[...]` — agents
            // hand-writing a single op shouldn't need the wrapper.
            let body = if raw.is_array() {
                serde_json::json!({ "ops": raw })
            } else {
                raw
            };
            let resp = post_json(
                &format!("{server}/api/hmi/{}/ops", url_encode(&path)),
                &body,
            )?;
            print_json(&resp)
        }
        HmiCmd::Check {
            path,
            server: ServerOpt { server },
        } => {
            let resp = get_json(&format!("{server}/api/hmi/{}/check", url_encode(&path)))?;
            print_json(&resp)
        }
        HmiCmd::Generate {
            path,
            force,
            title,
            server: ServerOpt { server },
        } => {
            let body = serde_json::json!({ "force": force, "title": title });
            let resp = post_json(
                &format!("{server}/api/hmi/{}/generate", url_encode(&path)),
                &body,
            )?;
            print_json(&resp)
        }
        HmiCmd::Symbols {
            server: ServerOpt { server },
        } => {
            let resp = get_json(&format!("{server}/api/hmi-symbols"))?;
            print_json(&resp)
        }
        HmiCmd::Delete {
            path,
            server: ServerOpt { server },
        } => {
            let resp = delete_json(&format!("{server}/api/hmi/{}", url_encode(&path)))?;
            print_json(&resp)
        }
    }
}
