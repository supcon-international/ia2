//! `cs agent` — explicit takeover-session enter / leave / wrap.

use anyhow::{Context, Result};

use crate::announce::{session_id, SESSION_ENV};
use crate::http::{http_agent, post_json, ServerOpt};
use crate::AgentCmd;

pub(crate) fn cmd_agent(cmd: AgentCmd) -> Result<i32> {
    match cmd {
        AgentCmd::Run {
            label,
            server: ServerOpt { server },
            cmd,
        } => cmd_agent_run(&label, &server, cmd),
        AgentCmd::Enter {
            label,
            server: ServerOpt { server },
        } => {
            let id = session_id().to_string();
            agent_session_start(&server, &id, &label)?;
            // Print the id on stdout so shell scripts can capture it:
            //   SESSION=$(cs agent enter --label ...)
            //   ...
            //   cs agent leave --id "$SESSION"
            println!("{id}");
            Ok(0)
        }
        AgentCmd::Leave {
            id,
            server: ServerOpt { server },
        } => {
            let target = id.or_else(|| std::env::var(SESSION_ENV).ok());
            let body = match target {
                Some(id) => serde_json::json!({ "id": id }),
                None => serde_json::json!({}),
            };
            let _ = post_json(&format!("{server}/api/agent/session/end"), &body)?;
            Ok(0)
        }
    }
}

fn cmd_agent_run(label: &str, server: &str, cmd: Vec<String>) -> Result<i32> {
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    if cmd.is_empty() {
        anyhow::bail!("cs agent run: expected a command after `--`");
    }

    // Generate session id. We reuse the same per-process id helper
    // the heartbeat path uses so a session id is comparable in logs
    // to a heartbeat session hint.
    let id = session_id().to_string();
    agent_session_start(server, &id, label)?;

    // Background heartbeat keeper. Every second, refresh the
    // session-side last_heartbeat so the server-side watchdog
    // (SESSION_TTL = 30s) doesn't age us out mid-execution.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_keeper = stop.clone();
    let server_owned = server.to_string();
    let id_for_keeper = id.clone();
    let keeper = std::thread::spawn(move || {
        while !stop_for_keeper.load(Ordering::Relaxed) {
            // Best-effort — short timeout, swallow errors. A failed
            // heartbeat only matters after SESSION_TTL of failures
            // in a row.
            let _ = http_agent()
                .post(&format!("{server_owned}/api/agent/heartbeat"))
                .timeout(std::time::Duration::from_millis(500))
                .set("Content-Type", "application/json")
                .send_json(serde_json::json!({
                    "command": null,
                    "session": id_for_keeper,
                }));
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });

    // Run the inner command. Expose the session id in its env so
    // any cs subcalls within `bash -c '...'` carry the same session.
    let mut child = Command::new(&cmd[0]);
    child
        .args(&cmd[1..])
        .env(SESSION_ENV, &id)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = child
        .status()
        .with_context(|| format!("spawning `{}`", cmd[0]))?;

    // Cleanup: stop the keeper, then close the session. Done in
    // try/finally style — even if the inner command crashed, we
    // close so the overlay doesn't get stuck on.
    stop.store(true, Ordering::Relaxed);
    let _ = keeper.join();
    let _ = post_json(
        &format!("{server}/api/agent/session/end"),
        &serde_json::json!({ "id": id }),
    );

    Ok(status.code().unwrap_or(1))
}

/// Open a session on the server. Errors propagate so the caller
/// can decide whether to still run the wrapped command — current
/// policy is "fail fast" since the user explicitly asked for
/// session-mode visual feedback.
fn agent_session_start(server: &str, id: &str, label: &str) -> Result<()> {
    let url = format!("{server}/api/agent/session/start");
    let resp = http_agent()
        .post(&url)
        .set("Content-Type", "application/json")
        .send_json(serde_json::json!({ "id": id, "label": label }))
        .map_err(|e| anyhow::anyhow!("POST {url}: {e}"))?;
    // Drain the body so the connection can be reused.
    let _: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
    Ok(())
}
