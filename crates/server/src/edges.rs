//! Edge orchestration: SSH-based probe, deploy, and attach.
//!
//! Trust model:
//!  - The dev machine has `ssh` available and resolves `host` via
//!    `~/.ssh/config` (keys, agent, jump hosts).
//!  - The edge box has `ia2-runtime` either pre-installed at
//!    `<install_dir>/current/runtime` (after `infra/install.sh`) or it's
//!    pushed by deploy.
//!  - Remote network access to the runtime's monitor server is **only**
//!    via the SSH port-forward set up by `attach`. The runtime always
//!    binds `127.0.0.1` on the edge.

use std::collections::HashMap;
use std::io;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use project::Edge;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use ts_rs::TS;

// ============================================================
//  Active attachments — port-forward + chosen local port
// ============================================================

/// One live `ssh -N -L` child plus the local port it's listening on.
struct ActiveAttachment {
    local_port: u16,
    /// Keeping the `Child` alive is the whole point — `kill_on_drop(true)`
    /// is what tears down the tunnel when the entry is removed/replaced.
    #[allow(dead_code)]
    child: Child,
}

#[derive(Default)]
pub struct AttachmentRegistry {
    /// Keyed by `(project_name, edge_name)`. Two projects with an
    /// identically-named edge keep separate tunnels; closing one
    /// doesn't touch the other. Replacing an entry drops the previous
    /// Child, which (because the runtime spawned it with
    /// `kill_on_drop(true)`) terminates the ssh tunnel cleanly.
    by_key: Mutex<HashMap<(String, String), ActiveAttachment>>,
}

impl AttachmentRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn current_port(&self, project_name: &str, edge_name: &str) -> Option<u16> {
        self.by_key
            .lock()
            .expect("attach registry")
            .get(&(project_name.to_string(), edge_name.to_string()))
            .map(|a| a.local_port)
    }

    pub fn insert(&self, project_name: String, edge_name: String, local_port: u16, child: Child) {
        self.by_key.lock().expect("attach registry").insert(
            (project_name, edge_name),
            ActiveAttachment { local_port, child },
        );
    }

    /// Stop the port-forward for one edge (if any). Returns whether
    /// something was actually running.
    pub fn detach(&self, project_name: &str, edge_name: &str) -> bool {
        let mut guard = self.by_key.lock().expect("attach registry");
        guard
            .remove(&(project_name.to_string(), edge_name.to_string()))
            .is_some()
    }

    /// Drop every tunnel attached to a given project. Called from
    /// `/api/projects/{name}/close` so closing a project tears down
    /// its tunnels without affecting other projects' tunnels.
    pub fn detach_all_for_project(&self, project_name: &str) {
        let mut guard = self.by_key.lock().expect("attach registry");
        guard.retain(|(p, _), _| p != project_name);
    }
}

// ============================================================
//  Probe — quick reachability + version snapshot
// ============================================================

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct EdgeProbe {
    /// `true` when the ssh + curl chain reached the runtime's `/health`.
    pub reachable: bool,
    /// Latest scan count from `/health`, if reachable.
    pub scan_count: Option<u64>,
    /// Uptime reported by the runtime's `/health`, if reachable.
    pub uptime_secs: Option<u64>,
    /// Runtime version (from `/status`), if reachable. Empty if probe
    /// stopped at the cheaper `/health` step.
    pub runtime_version: Option<String>,
    /// First line of stderr / error message when unreachable. Gives the
    /// user enough hint to fix `~/.ssh/config` or `install_dir`.
    pub error: Option<String>,
}

/// Run a short `ssh host curl 127.0.0.1:<port>/health` and parse the result.
/// 5s connect timeout so unreachable boxes don't block the UI for long.
pub async fn probe_edge(edge: &Edge) -> EdgeProbe {
    let cmd = format!(
        "curl --silent --max-time 3 http://127.0.0.1:{}/health",
        edge.runtime_port
    );
    let output = match ssh_cmd(edge).arg(cmd).output().await {
        Ok(out) => out,
        Err(e) => {
            return EdgeProbe {
                reachable: false,
                scan_count: None,
                uptime_secs: None,
                runtime_version: None,
                error: Some(format!("spawn ssh failed: {e}")),
            };
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return EdgeProbe {
            reachable: false,
            scan_count: None,
            uptime_secs: None,
            runtime_version: None,
            error: Some(first_line(&stderr).to_string()),
        };
    }

    let body = String::from_utf8_lossy(&output.stdout);
    #[derive(Deserialize)]
    struct Health {
        status: String,
        uptime_secs: u64,
        scan_count: u64,
    }
    let Ok(parsed) = serde_json::from_str::<Health>(&body) else {
        return EdgeProbe {
            reachable: false,
            scan_count: None,
            uptime_secs: None,
            runtime_version: None,
            error: Some(format!("unexpected body: {}", first_line(&body))),
        };
    };
    if parsed.status != "ok" {
        return EdgeProbe {
            reachable: false,
            scan_count: None,
            uptime_secs: None,
            runtime_version: None,
            error: Some(format!("runtime not ok: {}", parsed.status)),
        };
    }
    EdgeProbe {
        reachable: true,
        scan_count: Some(parsed.scan_count),
        uptime_secs: Some(parsed.uptime_secs),
        runtime_version: None,
        error: None,
    }
}

// ============================================================
//  Deploy — atomic versioned dir + symlink swap + systemctl restart
// ============================================================

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct DeployReport {
    pub ok: bool,
    /// Timestamped version directory created on the edge (e.g.
    /// `2026-05-12T08-30-00`).
    pub version: String,
    /// Tail of stdout/stderr from the remote script — useful for
    /// surfacing the "what just happened" to the user.
    pub log: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error("packaging project: {0}")]
    Pack(String),
    #[error("ssh: {0}")]
    Ssh(#[from] io::Error),
    #[error("remote script failed (exit {0}):\n{1}")]
    Remote(i32, String),
}

/// Deploy a project directory + optional runtime binary to one edge.
///
/// `project_dir`     filesystem path of the project on the dev machine.
/// `runtime_binary`  path to a built `ia2-runtime` binary
///                   for the edge's architecture. Optional — when None,
///                   the deploy reuses whatever binary is already under
///                   `<install_dir>/current/runtime`.
pub async fn deploy_to_edge(
    edge: &Edge,
    project_dir: &std::path::Path,
    runtime_binary: Option<&std::path::Path>,
) -> Result<DeployReport, DeployError> {
    // ---- Pack project (+ optional binary) into a tar stream ----
    // We `tar -cf -` locally and pipe to ssh's stdin so we never need a
    // temp file on either side. The script on the edge extracts to a
    // timestamped dir and atomically flips the symlink.
    let mut tar = Command::new("tar");
    tar.arg("-cf")
        .arg("-")
        .arg("-C")
        .arg(project_dir.parent().unwrap_or(project_dir))
        .arg(
            project_dir
                .file_name()
                .map(|n| n.to_owned())
                .ok_or_else(|| DeployError::Pack("project dir has no name".into()))?,
        );
    if let Some(bin) = runtime_binary {
        let bin = bin
            .canonicalize()
            .map_err(|e| DeployError::Pack(e.to_string()))?;
        tar.arg("-C")
            .arg(bin.parent().unwrap())
            .arg(bin.file_name().unwrap());
    }
    tar.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut tar_child = tar.spawn().map_err(|e| DeployError::Pack(e.to_string()))?;
    let mut tar_stdout = tar_child
        .stdout
        .take()
        .ok_or_else(|| DeployError::Pack("tar stdout missing".into()))?;

    // ---- ssh remote script ----
    let project_basename = project_dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| DeployError::Pack("project dir name not utf-8".into()))?;
    let binary_basename = runtime_binary
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(str::to_string);
    let script = remote_deploy_script(
        &edge.install_dir,
        project_basename,
        binary_basename.as_deref(),
    );

    let mut ssh = ssh_cmd(edge)
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut ssh_stdin = ssh.stdin.take().expect("ssh stdin");

    // Stream tar → ssh stdin while we wait.
    tokio::spawn(async move {
        let _ = tokio::io::copy(&mut tar_stdout, &mut ssh_stdin).await;
        let _ = ssh_stdin.shutdown().await;
    });

    let out = ssh.wait_with_output().await?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!(
        "{stdout}{}{stderr}",
        if !stderr.is_empty() {
            "\n--stderr--\n"
        } else {
            ""
        }
    );

    if !out.status.success() {
        return Err(DeployError::Remote(
            out.status.code().unwrap_or(-1),
            combined,
        ));
    }

    // The script prints `VERSION=<ts>` as its last informational line.
    let version = combined
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("VERSION="))
        .unwrap_or("?")
        .to_string();
    Ok(DeployReport {
        ok: true,
        version,
        log: combined,
    })
}

/// Build the shell snippet that runs on the edge. Reads the project
/// tarball from stdin, extracts into a timestamped version dir, swaps
/// the `current` symlink atomically (rename(2) of a temp symlink), and
/// restarts the systemd unit. Old versions are kept; rollback is just a
/// symlink swap.
fn remote_deploy_script(
    install_dir: &str,
    project_basename: &str,
    binary_basename: Option<&str>,
) -> String {
    let bin_swap = match binary_basename {
        Some(name) => format!(
            "if [ -f \"$DEST/{name}\" ]; then\n  chmod +x \"$DEST/{name}\"\n  mv \"$DEST/{name}\" \"$DEST/runtime\"\nfi\n",
            name = name,
        ),
        None => String::new(),
    };
    format!(
        r#"set -euo pipefail
INSTALL_DIR={install_dir:?}
PROJECT={project_basename:?}
TS=$(date -u +%Y-%m-%dT%H-%M-%SZ)
DEST="$INSTALL_DIR/versions/$TS"
mkdir -p "$DEST"
# Extract everything the dev machine streamed in.
tar -xf - -C "$DEST"
# If a project subdir was bundled, lift its contents up so the layout is
# always $DEST/project + $DEST/runtime (whether the binary was sent or not).
if [ -d "$DEST/$PROJECT" ] && [ "$PROJECT" != "project" ]; then
  mv "$DEST/$PROJECT" "$DEST/project"
fi
{bin_swap}# Carry forward the runtime binary from `current` if this deploy didn't ship one.
if [ ! -f "$DEST/runtime" ] && [ -f "$INSTALL_DIR/current/runtime" ]; then
  cp "$INSTALL_DIR/current/runtime" "$DEST/runtime"
fi
if [ ! -x "$DEST/runtime" ]; then
  echo "no runtime binary in $DEST and no prior current to copy from" >&2
  exit 2
fi
# Atomic symlink swap: rename(2) over an existing symlink is atomic on
# every Linux filesystem worth using.
TMPLINK="$INSTALL_DIR/.current.new"
ln -sfn "$DEST" "$TMPLINK"
mv -Tf "$TMPLINK" "$INSTALL_DIR/current"
echo "VERSION=$TS"
# Reload the unit if systemd is available; tolerate environments without
# systemd (handy in containers / tests).
if command -v systemctl >/dev/null 2>&1; then
  if systemctl is-enabled --quiet ia2 2>/dev/null; then
    sudo systemctl restart ia2 || systemctl --user restart ia2 || true
  else
    echo "(ia2.service not enabled — install it once via infra/install.sh)" >&2
  fi
fi
"#,
    )
}

// ============================================================
//  Attach — SSH port-forward + ephemeral local port
// ============================================================

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct AttachInfo {
    /// Local TCP port the IDE backend should proxy to.
    pub local_port: u16,
}

/// Start an `ssh -N -L 127.0.0.1:<local_port>:127.0.0.1:<edge.runtime_port> <host>`
/// and stash the child in `registry` keyed by `(project_name, edge.name)`.
/// Re-attaching the same `(project, edge)` pair while one is live
/// replaces the previous tunnel; an identically-named edge in a
/// different project is independent.
pub async fn attach_edge(
    project_name: &str,
    edge: &Edge,
    registry: &AttachmentRegistry,
) -> io::Result<AttachInfo> {
    // Pick an ephemeral local port by binding briefly then releasing it
    // back to the OS — ssh will grab it a moment later. Tiny race window;
    // for an MVP dev tool it's acceptable.
    let probe = TcpListener::bind("127.0.0.1:0").await?;
    let local_port = probe.local_addr()?.port();
    drop(probe);

    // Kill any previous tunnel for this (project, edge) pair first.
    registry.detach(project_name, &edge.name);

    let forward = format!("{local_port}:127.0.0.1:{}", edge.runtime_port);
    let mut child = ssh_cmd(edge)
        .arg("-N")
        .arg("-L")
        .arg(&forward)
        // Drop privileges: no PTY, no stdin/stdout (we're not running a
        // command), kill on drop so the tunnel goes away if the server
        // exits unexpectedly.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    // Briefly wait until the local port is actually accepting connections
    // — otherwise the UI's first /events probe races the tunnel and 502s.
    let mut ready = false;
    for _ in 0..30 {
        if tokio::net::TcpStream::connect(format!("127.0.0.1:{local_port}"))
            .await
            .is_ok()
        {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        // Bail early if ssh already died (bad host, auth failure, etc.).
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
    }
    if !ready {
        let mut err_buf = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut err_buf).await;
        }
        let _ = child.kill().await;
        return Err(io::Error::other(format!(
            "ssh port-forward to {host} never came up: {err}",
            host = edge.host,
            err = first_line(&err_buf)
        )));
    }

    registry.insert(
        project_name.to_string(),
        edge.name.clone(),
        local_port,
        child,
    );
    Ok(AttachInfo { local_port })
}

// ============================================================
//  Helpers
// ============================================================

/// Build the base ssh command with our usual options: explicit port,
/// optional user, connect timeout, BatchMode (so we never hang on a
/// password prompt — keys / agent only).
pub fn ssh_cmd(edge: &Edge) -> Command {
    let mut c = Command::new("ssh");
    c.arg("-p")
        .arg(edge.ssh_port.to_string())
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=5")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new");
    let target = if edge.ssh_user.is_empty() {
        edge.host.clone()
    } else {
        format!("{}@{}", edge.ssh_user, edge.host)
    };
    c.arg(target);
    c
}

fn first_line(s: &str) -> &str {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim_end()
}
