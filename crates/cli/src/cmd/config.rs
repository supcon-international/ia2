//! Project-config read/write bridges: iomap / tasks / northbound /
//! library. Each mirrors an HTTP get/put (or list/import/remove) pair.

use anyhow::Result;

use crate::http::{
    delete_json, get_json, post_json, print_json, put_json, read_json_blob, url_encode, ServerOpt,
};
use crate::{IomapCmd, LibraryCmd, NorthboundCmd, TasksCmd};

pub(crate) fn cmd_iomap(cmd: IomapCmd) -> Result<i32> {
    match cmd {
        IomapCmd::Get {
            server: ServerOpt { server },
        } => {
            let resp = get_json(&format!("{server}/api/iomap"))?;
            print_json(&resp)
        }
        IomapCmd::Set {
            from,
            server: ServerOpt { server },
        } => {
            let body = read_json_blob(&from)?;
            let resp = put_json(&format!("{server}/api/iomap"), &body)?;
            print_json(&resp)
        }
    }
}

pub(crate) fn cmd_tasks(cmd: TasksCmd) -> Result<i32> {
    match cmd {
        TasksCmd::Get {
            server: ServerOpt { server },
        } => {
            let resp = get_json(&format!("{server}/api/tasks"))?;
            print_json(&resp)
        }
        TasksCmd::Set {
            from,
            server: ServerOpt { server },
        } => {
            let body = read_json_blob(&from)?;
            let resp = put_json(&format!("{server}/api/tasks"), &body)?;
            print_json(&resp)
        }
    }
}

pub(crate) fn cmd_northbound(cmd: NorthboundCmd) -> Result<i32> {
    match cmd {
        NorthboundCmd::Get {
            server: ServerOpt { server },
        } => {
            let resp = get_json(&format!("{server}/api/northbound"))?;
            print_json(&resp)
        }
        NorthboundCmd::Set {
            from,
            server: ServerOpt { server },
        } => {
            let body = read_json_blob(&from)?;
            let resp = put_json(&format!("{server}/api/northbound"), &body)?;
            print_json(&resp)
        }
    }
}

pub(crate) fn cmd_library(cmd: LibraryCmd) -> Result<i32> {
    match cmd {
        LibraryCmd::List {
            json,
            server: ServerOpt { server },
        } => {
            let resp = get_json(&format!("{server}/api/library"))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else if let Some(arr) = resp.as_array() {
                // Concise table: name · version · import state.
                for l in arr {
                    let name = l.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let version = l.get("version").and_then(|v| v.as_str()).unwrap_or("?");
                    let files = l
                        .get("imported_files")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    match l.get("imported_version").and_then(|v| v.as_str()) {
                        Some(iv) => println!(
                            "{name}  v{version}  imported(v{iv}, {files} block{})",
                            if files == 1 { "" } else { "s" }
                        ),
                        None => println!("{name}  v{version}  (not imported)"),
                    }
                }
            }
            Ok(0)
        }
        LibraryCmd::Import {
            library,
            blocks,
            server: ServerOpt { server },
        } => {
            let body = serde_json::json!({ "library": library, "blocks": blocks });
            let resp = post_json(&format!("{server}/api/library/import"), &body)?;
            print_json(&resp)
        }
        LibraryCmd::Remove {
            name,
            server: ServerOpt { server },
        } => {
            let resp = delete_json(&format!("{server}/api/library/{}", url_encode(&name)))?;
            print_json(&resp)
        }
    }
}
