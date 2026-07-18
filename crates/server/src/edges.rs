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

use parking_lot::Mutex;
use std::collections::HashMap;
use std::io;
use std::process::Stdio;
use std::sync::Arc;

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
            .get(&(project_name.to_string(), edge_name.to_string()))
            .map(|a| a.local_port)
    }

    pub fn insert(&self, project_name: String, edge_name: String, local_port: u16, child: Child) {
        self.by_key.lock().insert(
            (project_name, edge_name),
            ActiveAttachment { local_port, child },
        );
    }

    /// Stop the port-forward for one edge (if any). Returns whether
    /// something was actually running.
    pub fn detach(&self, project_name: &str, edge_name: &str) -> bool {
        let mut guard = self.by_key.lock();
        guard
            .remove(&(project_name.to_string(), edge_name.to_string()))
            .is_some()
    }

    /// Drop every tunnel attached to a given project. Called from
    /// `/api/projects/{name}/close` so closing a project tears down
    /// its tunnels without affecting other projects' tunnels.
    pub fn detach_all_for_project(&self, project_name: &str) {
        let mut guard = self.by_key.lock();
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

/// systemd unit name for the edge runtime. The unit is the single source
/// of truth on the box for *where* the runtime listens (its `--bind` port)
/// and *which* project dir it runs — so when the configured port doesn't
/// answer we ask systemd rather than failing blind on one fixed port.
const EDGE_UNIT: &str = "ia2";

/// What the box's service manager reports about the runtime. Authoritative:
/// the edge config's `runtime_port` is only a fast-path hint, so a port/path
/// drift (or a stopped service) yields a real answer instead of a bare
/// connection failure.
#[derive(Debug, Default)]
struct ServiceState {
    /// `ActiveState` verbatim (`active`|`inactive`|`failed`|…). Empty when
    /// systemd or the unit isn't present (non-systemd edge / not installed).
    active_state: String,
    /// Port parsed from the unit's ExecStart `--bind 127.0.0.1:PORT`.
    bind_port: Option<u16>,
    /// `--project-dir` from ExecStart — lets deploy detect path drift.
    project_dir: Option<String>,
}

impl ServiceState {
    fn is_active(&self) -> bool {
        self.active_state == "active"
    }
}

/// Ask systemd on the edge about the runtime unit (one ssh round-trip).
/// `systemctl show` prints empty values for an unknown unit and still exits
/// 0, so we parse defensively and never error on "no such unit".
async fn query_service(edge: &Edge) -> ServiceState {
    let cmd =
        format!("systemctl show {EDGE_UNIT} -p ActiveState -p ExecStart --no-pager 2>/dev/null");
    let Ok(out) = ssh_cmd(edge).arg(cmd).output().await else {
        return ServiceState::default();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut st = ServiceState::default();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("ActiveState=") {
            st.active_state = v.trim().to_string();
        } else if line.starts_with("ExecStart=") {
            // The value embeds `argv[]=<bin> --project-dir <dir> --bind host:port ; …`.
            // Scan adjacent tokens for the two flags we care about.
            let toks: Vec<&str> = line.split_whitespace().collect();
            for w in toks.windows(2) {
                match w[0] {
                    "--bind" => st.bind_port = w[1].rsplit(':').next().and_then(|p| p.parse().ok()),
                    "--project-dir" => st.project_dir = Some(w[1].to_string()),
                    _ => {}
                }
            }
        }
    }
    st
}

/// Result of one `ssh host curl …` attempt, split so callers can tell
/// "the box is unreachable" (ssh) apart from "nothing is listening on that
/// port" (curl) — the two need very different remedies.
enum CurlOutcome {
    Body(String),
    /// curl ran but couldn't connect (e.g. exit 7) — runtime not on that port.
    NotListening,
    SshFailed(String),
}

async fn run_ssh_curl(edge: &Edge, remote_cmd: &str) -> CurlOutcome {
    let out = match ssh_cmd(edge).arg(remote_cmd).output().await {
        Ok(o) => o,
        Err(e) => return CurlOutcome::SshFailed(format!("spawn ssh: {e}")),
    };
    if out.status.success() {
        return CurlOutcome::Body(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    // ssh uses exit 255 for its own connect/auth failures; any other code is
    // the remote curl's (e.g. 7 = connection refused → nothing on that port).
    if out.status.code() == Some(255) {
        CurlOutcome::SshFailed(first_line(&String::from_utf8_lossy(&out.stderr)).to_string())
    } else {
        CurlOutcome::NotListening
    }
}

/// Reach the edge runtime over ssh+curl without hanging on one fixed port.
/// Strategy: try the configured port first (the fast common case); if
/// nothing answers there, consult systemd — is the service even running,
/// and what port did it actually bind? Returns the body or a layered,
/// actionable error. `make_cmd(port)` builds the remote curl command.
async fn edge_runtime_curl(
    edge: &Edge,
    make_cmd: impl Fn(u16) -> String,
) -> Result<String, String> {
    match run_ssh_curl(edge, &make_cmd(edge.runtime_port)).await {
        CurlOutcome::Body(b) => return Ok(b),
        CurlOutcome::SshFailed(e) => return Err(format!("ssh to {} failed: {e}", edge.host)),
        CurlOutcome::NotListening => {} // fall through to the source of truth
    }

    let svc = query_service(edge).await;
    if !svc.is_active() {
        let state = if svc.active_state.is_empty() {
            "not installed".to_string()
        } else {
            svc.active_state.clone()
        };
        return Err(format!(
            "runtime not reachable on {host}: systemd unit '{EDGE_UNIT}' is {state} \
             — start it with `sudo systemctl start {EDGE_UNIT}`",
            host = edge.host,
        ));
    }
    match svc.bind_port {
        // Active, but on a different port than configured — recover via the real one.
        Some(p) if p != edge.runtime_port => match run_ssh_curl(edge, &make_cmd(p)).await {
            CurlOutcome::Body(b) => Ok(b),
            _ => Err(format!(
                "'{EDGE_UNIT}' is active on {host} bound to :{p}, but the edge config has \
                 runtime_port={cfg} and neither answers — reconcile runtime_port with the unit",
                host = edge.host,
                cfg = edge.runtime_port,
            )),
        },
        _ => Err(format!(
            "'{EDGE_UNIT}' is active on {host} but not answering on :{} — health endpoint may be down",
            edge.runtime_port,
            host = edge.host,
        )),
    }
}

/// Probe the edge runtime's `/health` (port-resilient — see `edge_runtime_curl`).
pub async fn probe_edge(edge: &Edge) -> EdgeProbe {
    #[derive(Deserialize)]
    struct Health {
        status: String,
        uptime_secs: u64,
        scan_count: u64,
    }
    let unreachable = |error: String| EdgeProbe {
        reachable: false,
        scan_count: None,
        uptime_secs: None,
        runtime_version: None,
        error: Some(error),
    };
    let body = match edge_runtime_curl(edge, |port| {
        format!("curl --silent --max-time 3 http://127.0.0.1:{port}/health")
    })
    .await
    {
        Ok(b) => b,
        Err(e) => return unreachable(e),
    };
    let Ok(parsed) = serde_json::from_str::<Health>(&body) else {
        return unreachable(format!("unexpected body: {}", first_line(&body)));
    };
    if parsed.status != "ok" {
        return unreachable(format!("runtime not ok: {}", parsed.status));
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
//  Logs — pull recent runtime log lines over ssh+curl
// ============================================================

/// GET a JSON endpoint on the edge runtime (over ssh, same trust model
/// as `probe`) and return the body verbatim. One helper behind all the
/// read-side edge proxies:
///   `/logs?tail=N` — recent captured log lines (EtherCAT discovery,
///   bus health, connect errors that otherwise live only in journald);
///   `/discover` — per-device connect status + EtherCAT topology, so
///   the IDE can author PDO maps against the real bus;
///   `/system` — NICs / serial ports / arch, so device configs are
///   authored against real edge facts rather than guesses;
///   `/status` — project, scan count, debug mode/forces, and the last
///   VarSnapshot (with per-variable types, which `cs runtime --edge`
///   uses to pack force/write values).
pub async fn fetch_edge_json(
    edge: &Edge,
    path_and_query: &str,
) -> Result<serde_json::Value, String> {
    let body = edge_runtime_curl(edge, |port| {
        format!("curl --silent --max-time 4 'http://127.0.0.1:{port}{path_and_query}'")
    })
    .await?;
    serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
        format!(
            "unexpected {path_and_query} body: {} ({e})",
            first_line(&body)
        )
    })
}

// ============================================================
//  Online debug control — proxy pause/step/write/force to the edge
// ============================================================

/// POST a JSON body to an edge runtime control endpoint over ssh+curl
/// (`pause` / `resume` / `step` / `write` / `force` / `unforce`). The
/// caller must whitelist `path` — it's interpolated into the remote
/// command. Single quotes in the body are escaped for the shell.
pub async fn post_edge_runtime(
    edge: &Edge,
    path: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let body_str = body.to_string().replace('\'', r"'\''");
    let resp = edge_runtime_curl(edge, |port| {
        format!(
            "curl --silent --max-time 4 -X POST -H 'Content-Type: application/json' \
             -d '{body_str}' http://127.0.0.1:{port}/{path}"
        )
    })
    .await?;
    // The edge returns JSON on success; on a 4xx/5xx curl still exits 0
    // and the body is the plain-text error — surface that.
    serde_json::from_str::<serde_json::Value>(&resp)
        .map_err(|_| format!("edge runtime: {}", first_line(&resp)))
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
/// `web_dist`        built web assets (the IDE server's own
///                   `--static-dir`). Optional — when present they land
///                   at `<install_dir>/current/web` so the edge runtime
///                   can serve the standalone HMI panel; when None the
///                   remote script carries the previous version's `web/`
///                   forward (same rule as the binary).
pub async fn deploy_to_edge(
    edge: &Edge,
    project_dir: &std::path::Path,
    runtime_binary: Option<&std::path::Path>,
    web_dist: Option<&std::path::Path>,
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
    let web_basename = match web_dist {
        Some(dir) => {
            let dir = dir
                .canonicalize()
                .map_err(|e| DeployError::Pack(e.to_string()))?;
            let name = dir
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .ok_or_else(|| DeployError::Pack("web dist dir has no utf-8 name".into()))?;
            tar.arg("-C").arg(dir.parent().unwrap()).arg(&name);
            Some(name)
        }
        None => None,
    };
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
        web_basename.as_deref(),
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

    // Guard the classic drift: deploying to an `install_dir` the running
    // service doesn't actually read. systemd is the source of truth — if its
    // ExecStart runs from a different tree, this deploy is invisible to it.
    let mut combined = combined;
    let svc = query_service(edge).await;
    if let Some(svc_root) = svc
        .project_dir
        .as_deref()
        .map(|pd| pd.strip_suffix("/current/project").unwrap_or(pd))
    {
        if svc_root != edge.install_dir {
            combined = format!(
                "WARNING: deployed to install_dir={} but systemd '{EDGE_UNIT}' runs from {} — \
                 the service will NOT see this deploy. Reconcile the edge's install_dir with the \
                 unit's INSTALL_DIR.\n{}",
                edge.install_dir, svc_root, combined,
            );
        }
    }

    Ok(DeployReport {
        ok: true,
        version,
        log: combined,
    })
}

/// Quote `s` as a single shell word using single quotes, which disable
/// all expansion. Unlike Rust's `{:?}` Debug formatting — whose
/// double-quote escaping still lets `$(…)` / backticks run — this is safe
/// for interpolating arbitrary values into a remote shell script. An
/// embedded single quote is closed, escaped, and reopened (`'\''`), the
/// same trick `post_edge_runtime` uses for the curl body.
fn sh_squote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
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
    web_basename: Option<&str>,
) -> String {
    let bin_swap = match binary_basename {
        Some(name) => {
            let bin = sh_squote(name);
            format!(
                "if [ -f \"$DEST/\"{bin} ]; then\n  chmod +x \"$DEST/\"{bin}\n  mv \"$DEST/\"{bin} \"$DEST/runtime\"\nfi\n",
            )
        }
        None => String::new(),
    };
    // Normalise the bundled web assets (whatever their local dir was
    // called, usually `dist`) to `$DEST/web` — the fixed path the systemd
    // unit's `--static-dir` points at.
    let web_swap = match web_basename {
        Some(name) if name != "web" => {
            let web = sh_squote(name);
            format!("if [ -d \"$DEST/\"{web} ]; then\n  mv \"$DEST/\"{web} \"$DEST/web\"\nfi\n",)
        }
        _ => String::new(),
    };
    // Single-quote every value that lands in the remote script. `{:?}`
    // (Debug) is NOT shell quoting: it escapes `"`/`\` but leaves `$(…)`
    // and backticks live inside the resulting double-quoted assignment.
    let install_dir = sh_squote(install_dir);
    let project_basename = sh_squote(project_basename);
    format!(
        r#"set -euo pipefail
INSTALL_DIR={install_dir}
PROJECT={project_basename}
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
{bin_swap}{web_swap}# Carry forward the runtime binary from `current` if this deploy didn't ship one.
if [ ! -f "$DEST/runtime" ] && [ -f "$INSTALL_DIR/current/runtime" ]; then
  cp "$INSTALL_DIR/current/runtime" "$DEST/runtime"
fi
# Same for the HMI panel assets — a dev-server deploy (no dist) keeps
# whatever panel the edge already had.
if [ ! -d "$DEST/web" ] && [ -d "$INSTALL_DIR/current/web" ]; then
  cp -R "$INSTALL_DIR/current/web" "$DEST/web"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sh_squote_neutralizes_command_substitution() {
        assert_eq!(sh_squote("/opt/ia2"), "'/opt/ia2'");
        // `$(…)` and backticks are inert inside single quotes.
        assert_eq!(sh_squote("/opt/$(reboot)"), "'/opt/$(reboot)'");
        // an embedded single quote is closed, escaped, reopened.
        assert_eq!(sh_squote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn deploy_script_keeps_metachars_single_quoted() {
        let s = remote_deploy_script(
            "/opt/$(reboot)",
            "proj$(touch /tmp/x)",
            Some("rt`whoami`"),
            Some("dist$(id)"),
        );
        // Dangerous values appear only inside single-quoted words, so the
        // remote shell treats them as literals rather than evaluating them.
        assert!(s.contains("INSTALL_DIR='/opt/$(reboot)'"), "{s}");
        assert!(s.contains("PROJECT='proj$(touch /tmp/x)'"), "{s}");
        assert!(s.contains(r"'rt`whoami`'"), "{s}");
        assert!(s.contains("'dist$(id)'"), "{s}");
        // The pre-fix bug: the value inside a double-quoted assignment.
        assert!(!s.contains(r#"INSTALL_DIR="/opt/$(reboot)""#), "{s}");
    }

    #[test]
    fn deploy_script_normalises_web_assets_and_carries_forward() {
        let s = remote_deploy_script("/opt/ia2", "proj", None, Some("dist"));
        // Bundled `dist/` is renamed to the fixed `web/` the systemd
        // unit's --static-dir points at.
        assert!(s.contains(r#"mv "$DEST/"'dist' "$DEST/web""#), "{s}");
        // A deploy without web assets keeps the previous version's panel.
        assert!(
            s.contains(r#"cp -R "$INSTALL_DIR/current/web" "$DEST/web""#),
            "{s}"
        );
        // Already-named `web` needs no rename step.
        let s2 = remote_deploy_script("/opt/ia2", "proj", None, Some("web"));
        assert!(!s2.contains(r#""$DEST/"'web' "$DEST/web""#), "{s2}");
    }
}
