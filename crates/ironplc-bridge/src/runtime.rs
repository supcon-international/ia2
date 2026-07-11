//! Bridges the ironplc VM scan loop to:
//!  - `tokio::sync::broadcast` for streaming `VarSnapshot`s to subscribers.
//!  - `iocore::IoDevice` adapters for reading inputs before `run_round` and
//!    writing outputs after.
//!
//! The scan thread is a dedicated `std::thread` that hosts a single-thread
//! tokio runtime; everything bus-related runs inside it. ironplc's
//! `VmRunning::run_round` itself is sync.
//!
//! Multi-PROGRAM execution (ADR-0001): the one scan thread hosts N
//! `VmRunning` instances ("units"), one per scheduled PROGRAM instance,
//! each compiled into its own `Container`. Every unit keeps its own
//! cadence anchor from its task's interval; units due on the same tick
//! run in task-priority order (then tasks.toml declaration order).
//! Devices stay owned by the thread and are shared across units
//! sequentially — no concurrency on the bus.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::retain;
use iocore::{ChannelValue, IoDevice};
use ironplc_container::debug_format::{build_var_debug_map, format_variable_value, VarDebugInfo};
use ironplc_container::debug_section::iec_type_tag;
use ironplc_container::Container;
use ironplc_container::VarIndex;
use ironplc_container::STRING_HEADER_BYTES;
use ironplc_vm::{Vm, VmBuffers, VmRunning};
use project::{Direction, Mapping, ProtocolConfig};
use serde::Serialize;
use tokio::sync::broadcast;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct VarValue {
    pub name: String,
    pub type_name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct VarSnapshot {
    pub timestamp_us: u64,
    pub scan_count: u64,
    pub vars: Vec<VarValue>,
}

/// One subdevice on an EtherCAT bus, as reported by `/discover`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct DiscoveredSlave {
    pub index: u16,
    pub name: String,
    pub vendor_id: u32,
    pub product_id: u32,
    pub input_bytes: u16,
    pub output_bytes: u16,
}

/// Per-device connect outcome plus (for EtherCAT) the discovered bus
/// topology. Surfaced by the runtime's `/discover` endpoint so the IDE
/// can see which devices actually connected, why a connect failed, and
/// what's on the bus — the truth that otherwise only hits the logs.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct DeviceReport {
    pub name: String,
    /// `"modbus"` | `"ethercat"`.
    pub protocol: String,
    pub connected: bool,
    /// Connect error (first line) when `connected` is false.
    pub error: Option<String>,
    /// EtherCAT subdevices (empty for Modbus / failed connects).
    pub slaves: Vec<DiscoveredSlave>,
}

/// Live transport health of one configured device, refreshed by the scan
/// loop every round from [`IoDevice::is_healthy`]. This is the monitor-layer
/// consumer that flag was designed for: while a device's background link is
/// down its inputs keep serving the last-known mirror, and nothing in the
/// variable values themselves reveals that — only this flag does.
///
/// Devices that failed the initial connect appear here too, permanently
/// `healthy: false`, so a project configured for a device that never came
/// up is just as visible as one that dropped mid-run.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct DeviceHealth {
    pub name: String,
    /// `true` = transport believed good, variable values are live.
    /// `false` = connect failed or the background link is down; input
    /// variables are frozen at last-known values.
    pub healthy: bool,
}

#[derive(Clone, Debug)]
pub struct DeviceSpec {
    pub name: String,
    pub config: ProtocolConfig,
}

/// Reasons a variable write can't be honoured.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeWriteError {
    #[error("unknown variable '{0}' — not in the program's debug map")]
    UnknownVariable(String),
    #[error("scan loop has stopped — no writes will reach the VM")]
    Disconnected,
    #[error("vm trap during write: {0}")]
    Vm(String),
}

/// Out-of-band commands the scan loop drains between rounds. Used so
/// HTTP handlers (in the server's tokio runtime) can poke the VM (which
/// lives on a dedicated std::thread + current_thread runtime).
// The "Variable" postfix on every variant is intentional — these are
// commands that target a named runtime variable, and the postfix makes
// `RuntimeCommand::WriteVariable` self-documenting at the call site.
#[allow(clippy::enum_variant_names)]
enum RuntimeCommand {
    /// One-shot variable write — applied once, may be overwritten by
    /// the program in subsequent scans. Use `ForceVariable` to keep a
    /// value pinned.
    WriteVariable {
        name: String,
        value: i32,
        ack: tokio::sync::oneshot::Sender<Result<i32, RuntimeWriteError>>,
    },
    /// Pin a variable's value: every scan begins by writing `value`
    /// into the VM after the input phase but before `run_round`, so the
    /// program sees the forced value and field outputs reflect it.
    /// Calling Force again with the same name updates the value.
    ForceVariable {
        name: String,
        value: i32,
        ack: tokio::sync::oneshot::Sender<Result<i32, RuntimeWriteError>>,
    },
    /// Stop pinning a variable. The variable resumes normal program-
    /// driven behaviour from the next scan onwards. No-op if the name
    /// wasn't currently forced.
    UnforceVariable {
        name: String,
        ack: tokio::sync::oneshot::Sender<Result<(), RuntimeWriteError>>,
    },
}

/// Scan-loop execution mode. Mutated by the HTTP API / CLI via
/// `ProgramHandle::pause / resume / step`; the scan loop reads it at
/// the top of every round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeMode {
    /// Default — run continuously at the configured cycle rate.
    Running,
    /// Halt scan execution. IO inputs are NOT read, outputs are NOT
    /// written, the program does NOT advance. Variable writes /
    /// force commands still take effect (they're delivered between
    /// rounds, not inside a round), so an operator can manually
    /// stage a state while paused. This is "freeze the plant"
    /// semantics — safer than "freeze the program but keep the bus
    /// running".
    Paused,
    /// Run `remaining` scan cycles then automatically transition to
    /// `Paused`. Decremented atomically at the bottom of each cycle.
    Step { remaining: u32 },
}

/// Cheap-to-clone handle to a running scan loop. Multiple clones share the
/// same `stop` flag, snapshot fan-out, and command queue. Drop the last
/// clone to let resources go; explicit `.stop()` is preferred for clean
/// shutdown.
#[derive(Clone)]
pub struct ProgramHandle {
    stop: Arc<AtomicBool>,
    snapshot_tx: broadcast::Sender<VarSnapshot>,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<RuntimeCommand>,
    /// Shared mutable mode. `Arc<Mutex>` because the scan loop reads
    /// it once per cycle and the HTTP layer writes to it from an
    /// arbitrary tokio thread — a Mutex is cheap relative to a scan
    /// round, and atomicity over the `Step { remaining }` payload
    /// matters.
    mode: Arc<std::sync::Mutex<RuntimeMode>>,
    /// Currently-forced variables: name → pinned i32 value. The scan
    /// loop applies these in order on every cycle (after input read,
    /// before run_round). Mirrored here so the HTTP layer can return
    /// the active force set without a round-trip through the cmd
    /// queue.
    forces: Arc<std::sync::Mutex<HashMap<String, i32>>>,
    /// Per-device connect reports (connected/failed + EtherCAT topology),
    /// set once after the initial connect pass. Shared so the HTTP layer
    /// can serve /discover without a scan-loop round-trip.
    device_reports: Arc<std::sync::Mutex<Vec<DeviceReport>>>,
    /// Live per-device transport health, seeded after the connect pass and
    /// refreshed by the scan loop each round. Shared so the HTTP layer can
    /// serve health without a scan-loop round-trip.
    device_health: Arc<std::sync::Mutex<Vec<DeviceHealth>>>,
    /// Why the scan loop died, if it died: set exactly once — to the trap
    /// message when a VM trap stops the plant, or to the panic payload if
    /// the loop panicked — and never on a clean stop. A `watch` channel
    /// rather than a plain slot because the handle itself keeps the
    /// snapshot broadcast alive, so "stream closed" can never signal a
    /// fault — watchers must be woken by the fault WRITE itself. The
    /// sender lives on the scan thread; it drops (closing the watch) on
    /// clean exit.
    fault: tokio::sync::watch::Receiver<Option<String>>,
    /// The scan thread's join handle, shared so `shutdown` can join it
    /// from any clone. `Some` until the first `shutdown` takes it. Joining
    /// is how a caller waits for the always-run failsafe pass + per-device
    /// teardown (EtherCAT thread join) to actually finish.
    thread: Arc<std::sync::Mutex<Option<std::thread::JoinHandle<()>>>>,
}

impl ProgramHandle {
    /// Cooperative stop. The scan loop checks the flag at the top of each
    /// round; expect a few extra rounds before it actually exits.
    ///
    /// Fire-and-forget: returns immediately, doesn't wait for the failsafe
    /// pass. Use `shutdown` when you need the plant guaranteed safe before
    /// proceeding (clean process exit).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Graceful, awaitable stop for a clean process exit. Requests stop,
    /// then JOINS the scan thread — which runs its always-on failsafe pass
    /// (zero every device's outputs) and each device's `shutdown` (joining
    /// the EtherCAT cyclic worker so the zeroed controlword is on the wire)
    /// before it returns. When this completes, outputs are in failsafe.
    ///
    /// Unlike `stop`, this waits for completion, so the runtime can drive
    /// the plant safe before exiting rather than racing the service
    /// supervisor's kill timeout. The join runs on a blocking thread so
    /// this is safe to call from any async runtime. Idempotent: the first
    /// caller joins; later calls are no-ops.
    pub async fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let handle = self.thread.lock().ok().and_then(|mut g| g.take());
        let Some(handle) = handle else { return };
        match tokio::task::spawn_blocking(move || handle.join()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                tracing::error!(
                    "scan thread panicked during shutdown (outputs were failsafed first)"
                )
            }
            Err(e) => tracing::error!(%e, "failed to join scan thread on shutdown"),
        }
    }

    /// Subscribe to the per-cycle VarSnapshot stream.
    pub fn subscribe(&self) -> broadcast::Receiver<VarSnapshot> {
        self.snapshot_tx.subscribe()
    }

    /// Poke a variable while the program is running. Used by debug agents
    /// to force a state (toggle a setpoint, simulate an event flag, etc.).
    ///
    /// The write is applied between scan rounds, so it's seen by the next
    /// cycle's logic. Returns the value that was written; an error if the
    /// name doesn't resolve to a known variable or the VM traps.
    ///
    /// Name resolution (multi-PROGRAM runs): a bare name targets the
    /// first unit (tasks.toml declaration order) that declares it;
    /// `instance.variable` targets that PROGRAM instance explicitly
    /// (instance match is case-insensitive).
    pub async fn write_variable(&self, name: &str, value: i32) -> Result<i32, RuntimeWriteError> {
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        self.cmd_tx
            .send(RuntimeCommand::WriteVariable {
                name: name.to_string(),
                value,
                ack: ack_tx,
            })
            .map_err(|_| RuntimeWriteError::Disconnected)?;
        ack_rx.await.map_err(|_| RuntimeWriteError::Disconnected)?
    }

    /// Pin a variable to a fixed value. Until `unforce_variable` is
    /// called with the same name, the scan loop will write `value`
    /// back into the VM at the start of every cycle — so program
    /// writes get overridden each round and field inputs can't push
    /// through. Forces survive across scan cycles, unlike one-shot
    /// `write_variable`. Returns the value that was applied.
    ///
    /// Names resolve like `write_variable`: bare → first unit that has
    /// the variable, `instance.variable` → that unit explicitly.
    pub async fn force_variable(&self, name: &str, value: i32) -> Result<i32, RuntimeWriteError> {
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        self.cmd_tx
            .send(RuntimeCommand::ForceVariable {
                name: name.to_string(),
                value,
                ack: ack_tx,
            })
            .map_err(|_| RuntimeWriteError::Disconnected)?;
        ack_rx.await.map_err(|_| RuntimeWriteError::Disconnected)?
    }

    /// Release a forced variable. The variable resumes program-driven
    /// behaviour next scan. No-op if the variable wasn't forced.
    pub async fn unforce_variable(&self, name: &str) -> Result<(), RuntimeWriteError> {
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        self.cmd_tx
            .send(RuntimeCommand::UnforceVariable {
                name: name.to_string(),
                ack: ack_tx,
            })
            .map_err(|_| RuntimeWriteError::Disconnected)?;
        ack_rx.await.map_err(|_| RuntimeWriteError::Disconnected)?
    }

    /// List currently-forced (name, value) pairs. Cheap — reads from
    /// the shared map, no scan-loop round-trip.
    pub fn forces(&self) -> Vec<(String, i32)> {
        self.forces
            .lock()
            .ok()
            .map(|m| {
                let mut out: Vec<(String, i32)> = m.iter().map(|(k, &v)| (k.clone(), v)).collect();
                out.sort_by(|a, b| a.0.cmp(&b.0));
                out
            })
            .unwrap_or_default()
    }

    /// Read the current execution mode (Running / Paused / Step{N}).
    pub fn mode(&self) -> RuntimeMode {
        self.mode.lock().map(|m| *m).unwrap_or(RuntimeMode::Running)
    }

    /// Halt the scan loop. The current round finishes first; subsequent
    /// rounds are skipped until `resume` or `step` is called. IO is
    /// frozen (no inputs read, no outputs written) — `Paused` semantics
    /// match a "freeze the plant" debug stop, not "VM still ticking,
    /// outputs still flapping".
    pub fn pause(&self) {
        if let Ok(mut m) = self.mode.lock() {
            *m = RuntimeMode::Paused;
        }
    }

    /// Resume normal continuous scanning. No-op if already running.
    pub fn resume(&self) {
        if let Ok(mut m) = self.mode.lock() {
            *m = RuntimeMode::Running;
        }
    }

    /// Run `cycles` scan rounds then auto-pause. Calling `step` while
    /// in Step mode resets the remaining count.
    pub fn step(&self, cycles: u32) {
        if let Ok(mut m) = self.mode.lock() {
            *m = if cycles == 0 {
                RuntimeMode::Paused
            } else {
                RuntimeMode::Step { remaining: cycles }
            };
        }
    }

    /// Snapshot of per-device connect reports (connected/failed + EtherCAT
    /// topology). Empty until the initial connect pass completes.
    pub fn device_reports(&self) -> Vec<DeviceReport> {
        self.device_reports
            .lock()
            .map(|r| r.clone())
            .unwrap_or_default()
    }

    /// Live per-device transport health (see [`DeviceHealth`]). Empty until
    /// the initial connect pass completes. A device that failed to connect
    /// is reported permanently unhealthy; a connected device reflects its
    /// adapter's [`IoDevice::is_healthy`] as of the last scan round.
    pub fn device_health(&self) -> Vec<DeviceHealth> {
        self.device_health
            .lock()
            .map(|h| h.clone())
            .unwrap_or_default()
    }

    /// Why the scan loop died, if it died: `Some(message)` after a VM trap
    /// or a scan-thread panic, `None` while running or after a clean stop.
    /// Callers watching the snapshot stream use this at stream close to
    /// tell fault from stop.
    pub fn fault(&self) -> Option<String> {
        self.fault.borrow().clone()
    }

    /// Watch half of [`ProgramHandle::fault`], for callers that must be
    /// WOKEN on fault (the server's snapshot forwarder): `changed()`
    /// fires when the fault is written, and errors when the scan thread
    /// exits cleanly (sender dropped) — both are "stop watching" edges.
    pub fn fault_watch(&self) -> tokio::sync::watch::Receiver<Option<String>> {
        self.fault.clone()
    }

    /// True when `other` is a clone of this same spawned run — not merely a
    /// handle to an identically-shaped one. The server's stream forwarder
    /// uses this to release the program slot only if the slot still holds
    /// the run it was watching (a newer run may have replaced it).
    pub fn same_run(&self, other: &ProgramHandle) -> bool {
        Arc::ptr_eq(&self.stop, &other.stop)
    }
}

/// Default scan period when the caller doesn't request one. 100 ms
/// matches what Codesys-class tools default to and is plenty for
/// every demo we ship. Anything below ~10 ms starts taxing the
/// snapshot fan-out without giving the user faster perception.
pub const DEFAULT_SCAN_INTERVAL_MS: u64 = 100;

/// How often to flush retained variable values to disk during normal
/// operation. Power loss between flushes loses up to this much state.
/// 5 s strikes a balance between disk churn and worst-case data loss
/// for typical setpoints / counters / accumulators.
pub const RETAIN_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

/// One scheduled PROGRAM instance: its own compiled `Container` plus the
/// scheduling facts the scan thread needs to run it as an independent
/// "unit". Built by `ironplc_bridge::compile_project_units` (one per
/// `tasks.toml` program entry) or hand-rolled for ad-hoc single runs.
#[derive(Debug)]
pub struct ProgramUnit {
    /// PROGRAM instance name from tasks.toml (`PROGRAM <instance> WITH
    /// <task> : <program>;`). Used to route `Mapping.application`, to
    /// prefix colliding snapshot/retain names, and for the
    /// `instance.variable` write/force syntax.
    pub instance: String,
    /// Task this instance is bound to — informational (logs).
    pub task_name: String,
    /// Cycle period in milliseconds. `0` falls back to
    /// `DEFAULT_SCAN_INTERVAL_MS`.
    pub interval_ms: u64,
    /// IEC task priority — lower runs first when several units are due
    /// on the same tick.
    pub priority: i32,
    /// Bytecode compiled from this instance's PROGRAM + the project's
    /// FUNCTION_BLOCK / FUNCTION POUs + a synthesized single-task
    /// CONFIGURATION.
    pub container: Container,
    /// `VAR RETAIN` names declared by this unit's source.
    pub retain_vars: Vec<String>,
}

/// Start the scan thread hosting one `VmRunning` per unit.
///
/// Each unit is throttled to its own `interval_ms` regardless of what
/// (if anything) its compiled CONFIGURATION requested. Why the bridge
/// owns the cadence rather than the VM scheduler: as of the currently-
/// vendored ironplc, codegen does NOT populate `container.task_table`,
/// so the VM sees zero cyclic tasks and `next_due_us()` returns `None`.
/// Until upstream wires CONFIGURATION → task_table, the per-unit anchor
/// here is the source of truth for "the scan period the user asked for
/// in tasks.toml" (and `run_round` executes the unit's single PROGRAM
/// unconditionally each call).
///
/// RETAIN variables across all units persist into one state file when
/// `state_path` is set; keys are `instance.variable` when there is more
/// than one unit, bare names otherwise.
pub fn spawn_units(
    units: Vec<ProgramUnit>,
    device_specs: Vec<DeviceSpec>,
    mappings: Vec<Mapping>,
    state_path: Option<PathBuf>,
) -> ProgramHandle {
    spawn_units_inner(
        units,
        DeviceSource::Specs(device_specs),
        mappings,
        state_path,
    )
}

/// Where the scan thread gets its `IoDevice`s. Production always connects
/// from `DeviceSpec`s; tests inject pre-built devices so the failsafe /
/// shutdown sequencing can be asserted without real hardware.
enum DeviceSource {
    Specs(Vec<DeviceSpec>),
    #[cfg(test)]
    Prebuilt(Vec<Box<dyn IoDevice>>),
}

async fn acquire_devices(
    source: DeviceSource,
) -> (Vec<Box<dyn IoDevice>>, Vec<DeviceReport>, Vec<DeviceSpec>) {
    match source {
        DeviceSource::Specs(specs) => connect_devices(specs).await,
        #[cfg(test)]
        DeviceSource::Prebuilt(devices) => {
            let reports = devices
                .iter()
                .map(|d| DeviceReport {
                    name: d.name().to_string(),
                    protocol: "mock".into(),
                    connected: true,
                    error: None,
                    slaves: Vec::new(),
                })
                .collect();
            (devices, reports, Vec::new())
        }
    }
}

fn spawn_units_inner(
    units: Vec<ProgramUnit>,
    device_source: DeviceSource,
    mappings: Vec<Mapping>,
    state_path: Option<PathBuf>,
) -> ProgramHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let (snapshot_tx, _) = broadcast::channel(64);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let mode = Arc::new(std::sync::Mutex::new(RuntimeMode::Running));
    let forces = Arc::new(std::sync::Mutex::new(HashMap::<String, i32>::new()));
    let device_reports = Arc::new(std::sync::Mutex::new(Vec::<DeviceReport>::new()));
    let device_health = Arc::new(std::sync::Mutex::new(Vec::<DeviceHealth>::new()));
    let (fault_tx, fault_rx) = tokio::sync::watch::channel(None::<String>);

    let stop_clone = stop.clone();
    let stop_reconnect = stop.clone();
    let snapshot_tx_clone = snapshot_tx.clone();
    let mode_clone = mode.clone();
    let forces_clone = forces.clone();
    let device_reports_clone = device_reports.clone();
    let device_health_clone = device_health.clone();
    let device_reports_reconnect = device_reports.clone();

    let join_handle = std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!(%e, "failed to create scan-thread runtime");
                return;
            }
        };
        rt.block_on(async move {
            // Connect devices OUTSIDE the panic-guarded run_loop so
            // that, no matter how the loop exits (clean stop, VM trap,
            // or panic), we still own the `devices` vec and can drive
            // every output to its safe state.
            let (mut devices, reports, failed_specs) = acquire_devices(device_source).await;
            // Seed health from the connect outcomes: configured devices
            // that failed to connect start unhealthy (the per-round
            // refresh doesn't touch them until a background reconnect
            // brings them into `devices`); connected ones start from
            // their adapter's flag.
            if let Ok(mut slot) = device_health_clone.lock() {
                *slot = reports
                    .iter()
                    .map(|r| DeviceHealth {
                        name: r.name.clone(),
                        healthy: r.connected
                            && devices
                                .iter()
                                .find(|d| d.name() == r.name)
                                .map(|d| d.is_healthy())
                                .unwrap_or(false),
                    })
                    .collect();
            }
            if let Ok(mut slot) = device_reports_clone.lock() {
                *slot = reports;
            }

            // Devices that failed the initial connect are retried in the
            // background on a dedicated thread (own runtime, so a slow
            // EtherCAT bring-up can never stall scan rounds). Reconnected
            // adapters are handed to the scan loop over this channel; the
            // loop binds their parked mappings and the per-round health
            // refresh flips them healthy. If nothing failed, the sender
            // drops here and try_recv just reports Disconnected forever.
            let (reconnect_tx, reconnect_rx) =
                std::sync::mpsc::channel::<Box<dyn IoDevice>>();
            // Set once the failsafe + per-device shutdown pass below has
            // finished — the reconnect worker keeps its runtime (and the
            // adapters' background tasks) alive until then.
            let drained = Arc::new(AtomicBool::new(false));
            if !failed_specs.is_empty() {
                let drained_flag = drained.clone();
                let spawned = std::thread::Builder::new()
                    .name("ia2-reconnect".into())
                    .spawn(move || {
                        reconnect_worker(
                            failed_specs,
                            reconnect_tx,
                            device_reports_reconnect,
                            stop_reconnect,
                            drained_flag,
                        )
                    });
                if let Err(e) = spawned {
                    tracing::error!(%e, "failed to spawn reconnect thread; unconnected devices stay down");
                }
            }

            // Wrap the scan loop in catch_unwind so a panic in the VM
            // glue / iomap / snapshot fan-out doesn't skip failsafe.
            // `AssertUnwindSafe` is needed because `&mut Vec<...>` is
            // not auto-UnwindSafe; we accept that risk because the
            // only failure mode here is "panic in async code", and
            // we're about to discard the VMs anyway.
            use futures_util::FutureExt;
            use std::panic::AssertUnwindSafe;
            let result = AssertUnwindSafe(run_loop_async(
                units,
                &mut devices,
                mappings,
                stop_clone,
                snapshot_tx_clone,
                cmd_rx,
                mode_clone,
                forces_clone,
                device_health_clone,
                &fault_tx,
                reconnect_rx,
                state_path,
            ))
            .catch_unwind()
            .await;
            // A panic is a fault too: record it so the HTTP layer can
            // surface WHY the run died, not just that snapshots stopped.
            // (The failsafe + teardown below still run either way.)
            if let Err(panic) = &result {
                let msg = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "scan loop panicked".into());
                fault_tx.send_if_modified(|f| {
                    if f.is_none() {
                        *f = Some(format!("scan loop panicked: {msg}"));
                        true
                    } else {
                        false
                    }
                });
            }

            // Always-run failsafe before the thread dies. Drive every
            // device's outputs to zero so a hung / panicked / stopped
            // program doesn't leave actuators energized.
            let dev_count = devices.len();
            for dev in devices.iter_mut() {
                if let Err(e) = dev.enter_failsafe().await {
                    tracing::warn!(device = %dev.name(), %e, "failsafe call failed");
                }
            }
            // Give an async-flush device (real EtherCAT) a cycle or two to
            // push the zeros onto the bus. Conservative 50 ms covers
            // cycle_us up to 25 ms; paid once, on shutdown.
            tokio::time::sleep(Duration::from_millis(50)).await;
            // Graceful per-device teardown: signal + JOIN any background
            // I/O thread (the EtherCAT cyclic worker) so the zeroed
            // controlword is guaranteed on the wire before we exit — not
            // left to the drive's own watchdog after the master is gone.
            // Runs on both the clean and panicked paths (before re-panic).
            for dev in devices.iter_mut() {
                if let Err(e) = dev.shutdown().await {
                    tracing::warn!(device = %dev.name(), %e, "device shutdown failed");
                }
            }
            // Failsafe + teardown done: release the reconnect worker (it
            // holds the runtime that late-connected adapters' background
            // tasks live on, and must outlive the writes above).
            drained.store(true, Ordering::Relaxed);
            match &result {
                Ok(()) => tracing::info!(
                    devices = dev_count,
                    "scan loop exited cleanly; failsafe applied"
                ),
                Err(_) => tracing::error!(
                    devices = dev_count,
                    "scan loop PANICKED; failsafe applied before re-panic"
                ),
            }
            if let Err(panic) = result {
                // Re-raise so the thread dies with a useful backtrace
                // in tests / logs. Outputs are already safe.
                std::panic::resume_unwind(panic);
            }
        });
    });

    ProgramHandle {
        stop,
        snapshot_tx,
        cmd_tx,
        mode,
        forces,
        device_reports,
        device_health,
        fault: fault_rx,
        thread: Arc::new(std::sync::Mutex::new(Some(join_handle))),
    }
}

/// Connect every `DeviceSpec` into a live `IoDevice` adapter. A
/// connect failure for one device is logged and the device is skipped
/// rather than aborting the whole scan — partial bus connectivity is
/// a common operational state and we'd rather run the rest of the
/// program than refuse to start. The bridge's `enter_failsafe` pass
/// at shutdown only touches devices that DID connect.
async fn connect_devices(
    device_specs: Vec<DeviceSpec>,
) -> (Vec<Box<dyn IoDevice>>, Vec<DeviceReport>, Vec<DeviceSpec>) {
    let mut devices: Vec<Box<dyn IoDevice>> = Vec::with_capacity(device_specs.len());
    let mut reports: Vec<DeviceReport> = Vec::with_capacity(device_specs.len());
    let mut failed: Vec<DeviceSpec> = Vec::new();
    for spec in device_specs {
        let (device, report) = connect_one(&spec).await;
        match device {
            Some(d) => devices.push(d),
            None => failed.push(spec),
        }
        reports.push(report);
    }
    (devices, reports, failed)
}

/// Connect a single `DeviceSpec` into a live `IoDevice` adapter plus its
/// connect report. Shared by the initial connect pass and the background
/// reconnect worker so both paths produce identical devices and reports.
async fn connect_one(spec: &DeviceSpec) -> (Option<Box<dyn IoDevice>>, DeviceReport) {
    match &spec.config {
        ProtocolConfig::Modbus(cfg) => {
            match iomap_modbus::ModbusDevice::connect(spec.name.clone(), cfg).await {
                Ok(d) => {
                    // Log the transport-relevant detail so the
                    // operator sees "tcp 192.168.x.y:502" vs
                    // "rtu /dev/ttyUSB0 @ 9600" — same line
                    // pattern, transport-specific payload.
                    match &cfg.transport {
                        project::ModbusTransport::Tcp(p) => {
                            tracing::info!(name = %spec.name, transport = "tcp", host = %p.host, port = p.port, "modbus connected");
                        }
                        project::ModbusTransport::Rtu(p) => {
                            tracing::info!(
                                name = %spec.name,
                                transport = "rtu",
                                device = %p.serial_device,
                                baud = p.baud_rate,
                                "modbus connected"
                            );
                        }
                    }
                    (
                        Some(Box::new(d) as Box<dyn IoDevice>),
                        DeviceReport {
                            name: spec.name.clone(),
                            protocol: "modbus".into(),
                            connected: true,
                            error: None,
                            slaves: Vec::new(),
                        },
                    )
                }
                Err(e) => {
                    tracing::warn!(name = %spec.name, %e, "modbus connect failed");
                    (
                        None,
                        DeviceReport {
                            name: spec.name.clone(),
                            protocol: "modbus".into(),
                            connected: false,
                            error: Some(e.to_string()),
                            slaves: Vec::new(),
                        },
                    )
                }
            }
        }
        ProtocolConfig::Ethercat(cfg) => {
            // The EtherCAT bring-up (init_single_group / DC sync) can
            // lose the first PDU right after a fresh link/bind and fail
            // with a transient Timeout(Pdu). A failed connect() exits its
            // worker thread cleanly, so retry a few times with a short
            // backoff — otherwise one transient timeout leaves the bus
            // (and the motor) dead until a manual restart.
            const MAX_ATTEMPTS: u32 = 3;
            let mut attempt: u32 = 1;
            let connected = loop {
                match iomap_ethercat::EthercatDevice::connect(spec.name.clone(), cfg).await {
                    Ok(d) => break Ok(d),
                    Err(e) if attempt < MAX_ATTEMPTS => {
                        tracing::warn!(
                            name = %spec.name, attempt, max = MAX_ATTEMPTS, %e,
                            "ethercat connect failed; retrying after backoff"
                        );
                        tokio::time::sleep(Duration::from_millis(800)).await;
                        attempt += 1;
                    }
                    Err(e) => break Err(e),
                }
            };
            match connected {
                Ok(d) => {
                    tracing::info!(name = %spec.name, nic = %cfg.nic, "ethercat connected");
                    // Pull the discovered topology before boxing the
                    // device into the trait object.
                    let slaves = d
                        .discovered()
                        .into_iter()
                        .map(|s| DiscoveredSlave {
                            index: s.index,
                            name: s.name,
                            vendor_id: s.vendor_id,
                            product_id: s.product_id,
                            input_bytes: s.input_bytes,
                            output_bytes: s.output_bytes,
                        })
                        .collect();
                    (
                        Some(Box::new(d) as Box<dyn IoDevice>),
                        DeviceReport {
                            name: spec.name.clone(),
                            protocol: "ethercat".into(),
                            connected: true,
                            error: None,
                            slaves,
                        },
                    )
                }
                Err(e) => {
                    tracing::warn!(name = %spec.name, %e, "ethercat connect failed");
                    (
                        None,
                        DeviceReport {
                            name: spec.name.clone(),
                            protocol: "ethercat".into(),
                            connected: false,
                            error: Some(e.to_string()),
                            slaves: Vec::new(),
                        },
                    )
                }
            }
        }
        ProtocolConfig::Opcua(cfg) => {
            match iomap_opcua::OpcuaDevice::connect(spec.name.clone(), cfg).await {
                Ok(d) => {
                    tracing::info!(
                        name = %spec.name,
                        endpoint = %cfg.endpoint_url,
                        tags = cfg.channels.len(),
                        "opcua connected"
                    );
                    (
                        Some(Box::new(d) as Box<dyn IoDevice>),
                        DeviceReport {
                            name: spec.name.clone(),
                            protocol: "opcua".into(),
                            connected: true,
                            error: None,
                            slaves: Vec::new(),
                        },
                    )
                }
                Err(e) => {
                    tracing::warn!(name = %spec.name, %e, "opcua connect failed");
                    (
                        None,
                        DeviceReport {
                            name: spec.name.clone(),
                            protocol: "opcua".into(),
                            connected: false,
                            error: Some(e.to_string()),
                            slaves: Vec::new(),
                        },
                    )
                }
            }
        }
    }
}

/// Background retry loop for devices that failed the initial connect.
///
/// Runs on its own OS thread with a dedicated single-thread runtime so a
/// slow bus bring-up (EtherCAT walk, DC sync, TCP timeouts) can never
/// stall scan rounds. Retries every pending spec with a shared doubling
/// backoff (1 s → 30 s cap); each success rewrites that device's
/// /discover report in place and hands the live adapter to the scan
/// loop, which binds the mappings it parked at startup. Exits when every
/// spec has connected or the program stops (checked in ≤200 ms slices so
/// shutdown isn't held up by a long backoff).
fn reconnect_worker(
    mut pending: Vec<DeviceSpec>,
    tx: std::sync::mpsc::Sender<Box<dyn IoDevice>>,
    reports: Arc<std::sync::Mutex<Vec<DeviceReport>>>,
    stop: Arc<AtomicBool>,
    drained: Arc<AtomicBool>,
) {
    const BACKOFF_START: Duration = Duration::from_secs(1);
    const BACKOFF_CAP: Duration = Duration::from_secs(30);
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(%e, "failed to create reconnect runtime; unconnected devices stay down");
            return;
        }
    };
    // Everything runs inside one block_on: adapters connected here spawn
    // their background tasks (Modbus poll loop, OPC-UA session) onto THIS
    // runtime, and a current-thread runtime only makes progress while it's
    // being driven — so the worker must keep awaiting for as long as any
    // device it delivered is in service.
    rt.block_on(async move {
        tracing::warn!(
            devices = pending.len(),
            "entering background reconnect for devices that failed to connect"
        );
        let mut delivered_any = false;
        let mut backoff = BACKOFF_START;
        while !pending.is_empty() && !stop.load(Ordering::Relaxed) {
            let deadline = Instant::now() + backoff;
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                tokio::time::sleep(remaining.min(Duration::from_millis(200))).await;
            }
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let mut still_pending = Vec::with_capacity(pending.len());
            for spec in pending {
                let (device, report) = connect_one(&spec).await;
                match device {
                    Some(d) => {
                        tracing::info!(device = %spec.name, "background reconnect succeeded");
                        if let Ok(mut slot) = reports.lock() {
                            if let Some(r) = slot.iter_mut().find(|r| r.name == report.name) {
                                *r = report;
                            }
                        }
                        // Scan loop gone ⇒ nobody left to adopt the device.
                        if tx.send(d).is_err() {
                            return;
                        }
                        delivered_any = true;
                    }
                    None => still_pending.push(spec),
                }
            }
            pending = still_pending;
            backoff = (backoff * 2).min(BACKOFF_CAP);
        }
        if pending.is_empty() {
            tracing::info!("background reconnect complete; all configured devices connected");
        }
        if !delivered_any {
            return; // nothing spawned onto this runtime; safe to wind down
        }
        // Keep driving the delivered adapters' background tasks until the
        // scan thread reports its failsafe/shutdown pass is DONE (`drained`).
        // Exiting on `stop` alone would race that pass: dropping this
        // runtime kills the poll tasks the failsafe writes go through.
        while !drained.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });
}

/// Trip the watchdog after this many consecutive scan deadline overruns
/// on any single unit. Each overrun means that unit's scan body didn't
/// finish within its interval. 5 in a row → the simulation has lost
/// real-time guarantees; engage failsafe and don't re-arm until the
/// program is restarted.
const WATCHDOG_OVERRUN_THRESHOLD: u32 = 5;

/// Snapshot fan-out cadence (and the cap on idle sleeps, so stop /
/// command latency stays bounded even when every task interval is long).
const SNAPSHOT_PERIOD: Duration = Duration::from_millis(100);

/// A bus channel ↔ VM variable pair, resolved to indices and routed to
/// one unit.
struct ResolvedMapping {
    device_index: usize,
    channel: String,
    var_index: u16,
    type_tag: u8,
    /// REAL var — channel values cross the VM boundary as IEEE-754
    /// bits instead of numeric i32 (see `ChannelValue::to_vm_bits`).
    is_real: bool,
    /// LREAL var — crosses as a full 64-bit double via
    /// `write_variable_raw` (the i32 lane would truncate).
    is_lreal: bool,
}

/// Resolve one iomap `Mapping` against a connected device: route it to a
/// PROGRAM unit and produce the index-level `ResolvedMapping` the scan
/// loop consumes. Shared by the startup resolution pass and the mid-run
/// bind that happens when a device joins after a background reconnect.
///
/// Routing: by PROGRAM instance name (`Mapping.application`,
/// case-insensitive). An empty/unknown application falls back to the
/// first unit that declares the variable — the exact pre-multi-program
/// resolution, so legacy iomaps keep working; warns only when there's
/// real ambiguity (>1 unit). Returns `None` (with a warn) when the
/// variable can't be routed at all.
fn resolve_mapping(
    m: &Mapping,
    device_index: usize,
    instances: &[String],
    var_index_by_name: &[HashMap<String, u16>],
    debug_maps: &[HashMap<u16, VarDebugInfo>],
) -> Option<(usize, Direction, ResolvedMapping)> {
    let n_units = instances.len();
    let unit_index = match instances
        .iter()
        .position(|i| i.eq_ignore_ascii_case(&m.application))
    {
        Some(i) => {
            if !var_index_by_name[i].contains_key(&m.variable) {
                tracing::warn!(
                    application = %m.application,
                    var = %m.variable,
                    "mapping's variable not declared by its routed instance, skipping"
                );
                return None;
            }
            i
        }
        None => {
            let Some(i) = (0..n_units).find(|&i| var_index_by_name[i].contains_key(&m.variable))
            else {
                tracing::warn!(var = %m.variable, "mapping references unknown variable, skipping");
                return None;
            };
            if n_units > 1 {
                tracing::warn!(
                    application = %m.application,
                    var = %m.variable,
                    routed_to = %instances[i],
                    "mapping's application doesn't name a PROGRAM instance; \
                     falling back to the first unit that declares the variable"
                );
            }
            i
        }
    };
    let var_index = var_index_by_name[unit_index][&m.variable];
    let type_tag = debug_maps[unit_index]
        .get(&var_index)
        .map(|d| d.iec_type_tag)
        .unwrap_or(0);
    Some((
        unit_index,
        m.direction,
        ResolvedMapping {
            device_index,
            channel: m.channel.clone(),
            var_index,
            type_tag,
            is_real: type_tag == iec_type_tag::REAL,
            is_lreal: type_tag == iec_type_tag::LREAL,
        },
    ))
}

/// Per-unit scheduling state. Lives in a Vec parallel to `runnings`
/// (the `VmRunning`s borrow their containers, so unit bookkeeping stays
/// in plain owned data).
struct UnitClock {
    /// When this unit's next scan is due. Anchored, not drifting:
    /// advances by `interval` after each run; slides to `now +
    /// interval` on overrun so we don't burn CPU catching up.
    next_due: Instant,
    interval: Duration,
    scan_count: u64,
    consecutive_overruns: u32,
    warned_overrun: bool,
}

/// Resolve a runtime variable reference against all units. A bare name
/// matches the first unit (tasks.toml declaration order) that declares
/// it — which is also exactly the pre-multi-program behaviour when
/// there's one unit. `instance.variable` (instance match case-
/// insensitive) targets that unit explicitly. Bare lookup runs first so
/// a literal debug name containing a dot keeps resolving as it did
/// before multi-program support.
fn resolve_var(
    name: &str,
    instances: &[String],
    var_index_by_name: &[HashMap<String, u16>],
) -> Option<(usize, u16)> {
    for (i, m) in var_index_by_name.iter().enumerate() {
        if let Some(&idx) = m.get(name) {
            return Some((i, idx));
        }
    }
    let (prefix, rest) = name.split_once('.')?;
    let i = instances
        .iter()
        .position(|inst| inst.eq_ignore_ascii_case(prefix))?;
    var_index_by_name[i].get(rest).map(|&idx| (i, idx))
}

#[allow(clippy::too_many_arguments)]
async fn run_loop_async(
    units: Vec<ProgramUnit>,
    // Devices are borrowed so the outer wrapper retains ownership for
    // its always-run failsafe pass (see `spawn_units_inner`'s async
    // block).
    devices: &mut Vec<Box<dyn IoDevice>>,
    mappings: Vec<Mapping>,
    stop: Arc<AtomicBool>,
    snapshot_tx: broadcast::Sender<VarSnapshot>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<RuntimeCommand>,
    mode: Arc<std::sync::Mutex<RuntimeMode>>,
    forces: Arc<std::sync::Mutex<HashMap<String, i32>>>,
    device_health: Arc<std::sync::Mutex<Vec<DeviceHealth>>>,
    // Borrowed: the wrapper keeps ownership so its panic path can also
    // record into the same channel (and so the sender drops — closing
    // the watch — only when the scan thread exits).
    fault: &tokio::sync::watch::Sender<Option<String>>,
    reconnect_rx: std::sync::mpsc::Receiver<Box<dyn IoDevice>>,
    state_path: Option<PathBuf>,
) {
    if units.is_empty() {
        tracing::error!("no program units to run — scan loop not started");
        return;
    }
    let n_units = units.len();

    // ---- Start one VM per unit ----
    // `VmRunning` borrows its container and its buffers, so both live
    // in Vecs that outlive `runnings` and are never structurally
    // touched again (`iter_mut` hands out disjoint element borrows).
    let mut bufs: Vec<VmBuffers> = units
        .iter()
        .map(|u| VmBuffers::from_container(&u.container))
        .collect();
    let mut runnings: Vec<VmRunning<'_>> = Vec::with_capacity(n_units);
    for (unit, buf) in units.iter().zip(bufs.iter_mut()) {
        match Vm::new().load(&unit.container, buf).start() {
            Ok(r) => runnings.push(r),
            Err(ctx) => {
                tracing::error!(instance = %unit.instance, ?ctx.trap, "vm failed to start");
                return;
            }
        }
    }

    let debug_maps: Vec<HashMap<u16, VarDebugInfo>> = units
        .iter()
        .map(|u| build_var_debug_map(&u.container))
        .collect();
    // Parallel to debug_maps: per-unit `var_index -> data_offset` for STRING
    // vars, whose bytes live in the VM data region rather than the slot.
    let string_layouts: Vec<HashMap<u16, u32>> = units
        .iter()
        .map(|u| build_string_layout_map(&u.container))
        .collect();
    let var_index_by_name: Vec<HashMap<String, u16>> = debug_maps
        .iter()
        .map(|dm| {
            dm.iter()
                .map(|(idx, info)| (info.name.clone(), *idx))
                .collect()
        })
        .collect();
    let instances: Vec<String> = units.iter().map(|u| u.instance.clone()).collect();

    // Variable names declared by more than one unit get the
    // `instance.` prefix in snapshots (and retain keys) so they stay
    // distinguishable; unique names stay bare — zero churn for
    // single-program projects.
    let shared_names: std::collections::HashSet<String> = if n_units > 1 {
        let mut counts: HashMap<&String, u32> = HashMap::new();
        for m in &var_index_by_name {
            for name in m.keys() {
                *counts.entry(name).or_insert(0) += 1;
            }
        }
        counts
            .into_iter()
            .filter(|&(_, c)| c > 1)
            .map(|(n, _)| n.clone())
            .collect()
    } else {
        Default::default()
    };

    // ---- Resolve mappings into index pairs, routed per unit ----
    let device_index_by_name: HashMap<String, usize> = devices
        .iter()
        .enumerate()
        .map(|(i, d)| (d.name().to_string(), i))
        .collect();

    let mut unit_inputs: Vec<Vec<ResolvedMapping>> = (0..n_units).map(|_| Vec::new()).collect();
    let mut unit_outputs: Vec<Vec<ResolvedMapping>> = (0..n_units).map(|_| Vec::new()).collect();
    // Mappings whose device isn't connected are parked, not dropped: the
    // background reconnect worker may still deliver that device mid-run,
    // at which point they bind exactly like the startup ones below.
    let mut pending_mappings: Vec<Mapping> = Vec::new();
    for m in mappings {
        let Some(&device_index) = device_index_by_name.get(&m.device) else {
            tracing::warn!(
                device = %m.device,
                "mapping references a device that isn't connected; parked until it reconnects"
            );
            pending_mappings.push(m);
            continue;
        };
        if let Some((unit_index, direction, rm)) = resolve_mapping(
            &m,
            device_index,
            &instances,
            &var_index_by_name,
            &debug_maps,
        ) {
            match direction {
                Direction::Input => unit_inputs[unit_index].push(rm),
                Direction::Output => unit_outputs[unit_index].push(rm),
            }
        }
    }
    for (i, unit) in units.iter().enumerate() {
        tracing::info!(
            instance = %unit.instance,
            task = %unit.task_name,
            interval_ms = unit.interval_ms,
            priority = unit.priority,
            inputs = unit_inputs[i].len(),
            outputs = unit_outputs[i].len(),
            retain_vars = unit.retain_vars.len(),
            "unit scheduled"
        );
    }
    tracing::info!(
        units = n_units,
        devices = devices.len(),
        state_path = ?state_path,
        "scan loop ready"
    );

    // ---- Resolve RETAIN names → (unit, state-file key, var index) ----
    // Keys are `instance.variable` when several units run (so same-
    // named retain vars in different units don't collide on disk) and
    // bare names for the single-unit case — the historical format.
    let mut retain_entries: Vec<(usize, String, u16)> = Vec::new();
    for (i, unit) in units.iter().enumerate() {
        for name in &unit.retain_vars {
            match var_index_by_name[i].get(name) {
                Some(&idx) => {
                    let key = if n_units > 1 {
                        format!("{}.{}", unit.instance, name)
                    } else {
                        name.clone()
                    };
                    retain_entries.push((i, key, idx));
                }
                None => {
                    // Possible if the user removed a RETAIN var from
                    // source between runs but the state file still
                    // references it. The next save rewrites the file
                    // without the stale entry.
                    tracing::warn!(instance = %unit.instance, var = %name, "retain var not in debug map; skipping");
                }
            }
        }
    }

    // ---- Restore RETAIN values from disk before scanning starts ----
    if !retain_entries.is_empty() {
        if let Some(path) = state_path.as_ref() {
            match retain::load(path) {
                Ok(Some(state)) => {
                    let mut restored = 0;
                    for (i, key, idx) in &retain_entries {
                        // Prefer the canonical key; accept the bare name
                        // as migration for state files written before
                        // the project became multi-PROGRAM.
                        let value = state.vars.get(key).copied().or_else(|| {
                            key.split_once('.')
                                .and_then(|(_, bare)| state.vars.get(bare).copied())
                        });
                        if let Some(value) = value {
                            // Raw 64-bit slot write — lossless for all
                            // IEC types (schema 2; v1 files arrive here
                            // pre-widened by retain::load's migration).
                            if runnings[*i]
                                .write_variable_raw(VarIndex::new(*idx), value)
                                .is_ok()
                            {
                                restored += 1;
                            } else {
                                tracing::warn!(var = %key, "restore write trapped; skipping");
                            }
                        }
                    }
                    tracing::info!(
                        restored,
                        total = retain_entries.len(),
                        saved_at_us = state.saved_at_us,
                        "restored retain variables from state file"
                    );
                }
                Ok(None) => {
                    tracing::info!(?path, "no retain state file yet; starting fresh");
                }
                Err(e) => {
                    // Don't refuse to start on a corrupt state file —
                    // log and continue with defaults. Operator can
                    // inspect or delete the file out-of-band.
                    tracing::warn!(?path, %e, "failed to read retain state file; using defaults");
                }
            }
        }
    }

    // ---- Scan loop ----
    let start = Instant::now();
    let mut last_snapshot = Instant::now() - Duration::from_secs(1);
    let mut last_retain_flush = Instant::now();
    // Every unit is due immediately on the first tick (matches the old
    // single-unit loop, which ran its first scan right away).
    let mut clocks: Vec<UnitClock> = units
        .iter()
        .map(|u| UnitClock {
            next_due: start,
            interval: Duration::from_millis(
                if u.interval_ms == 0 {
                    DEFAULT_SCAN_INTERVAL_MS
                } else {
                    u.interval_ms
                }
                .max(1),
            ),
            scan_count: 0,
            consecutive_overruns: 0,
            warned_overrun: false,
        })
        .collect();
    // Same-tick execution order: task priority (lower runs first),
    // then tasks.toml declaration order.
    let mut exec_order: Vec<usize> = (0..n_units).collect();
    exec_order.sort_by_key(|&i| (units[i].priority, i));
    // Watchdog: any unit accumulating WATCHDOG_OVERRUN_THRESHOLD
    // consecutive overruns fires failsafe once and disarms — the loop
    // keeps scanning so operators can see live state, but outputs stay
    // safe until the program is restarted (industrial convention).
    let mut watchdog_armed = true;
    let mut prev_paused = false;
    let mut vm_fault = false;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // Adopt any devices the background reconnect worker delivered:
        // append to the device set and bind the mappings that were parked
        // at startup because this device wasn't there. Runs before the
        // health refresh so the adopted device flips healthy this round.
        while let Ok(dev) = reconnect_rx.try_recv() {
            let name = dev.name().to_string();
            let device_index = devices.len();
            devices.push(dev);
            let mut bound = 0usize;
            let mut parked = Vec::with_capacity(pending_mappings.len());
            for m in pending_mappings.drain(..) {
                if m.device != name {
                    parked.push(m);
                    continue;
                }
                if let Some((unit_index, direction, rm)) = resolve_mapping(
                    &m,
                    device_index,
                    &instances,
                    &var_index_by_name,
                    &debug_maps,
                ) {
                    match direction {
                        Direction::Input => unit_inputs[unit_index].push(rm),
                        Direction::Output => unit_outputs[unit_index].push(rm),
                    }
                    bound += 1;
                }
            }
            pending_mappings = parked;
            tracing::info!(
                device = %name,
                mappings = bound,
                "device joined after background reconnect; its variables are live"
            );
        }

        // Mirror each live device's transport health for the monitor layer.
        // Cheap: one atomic load per device, and the lock is uncontended
        // except when an HTTP handler snapshots it. Entries for devices
        // that never connected aren't in `devices` and stay `false`.
        if let Ok(mut health) = device_health.lock() {
            for entry in health.iter_mut() {
                if let Some(d) = devices.iter().find(|d| d.name() == entry.name) {
                    entry.healthy = d.is_healthy();
                }
            }
        }

        // Drain any pending out-of-band commands (variable writes,
        // forces, unforces). Done EVERY iteration including when
        // paused — that's the whole point of pausing, you want to
        // poke values without scanning.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                RuntimeCommand::WriteVariable { name, value, ack } => {
                    let result = match resolve_var(&name, &instances, &var_index_by_name) {
                        Some((u, idx)) => {
                            match runnings[u].write_variable(VarIndex::new(idx), value) {
                                Ok(()) => Ok(value),
                                Err(trap) => Err(RuntimeWriteError::Vm(format!("{trap:?}"))),
                            }
                        }
                        None => Err(RuntimeWriteError::UnknownVariable(name)),
                    };
                    let _ = ack.send(result);
                }
                RuntimeCommand::ForceVariable { name, value, ack } => {
                    // Reject unknown names early so the caller gets
                    // immediate feedback instead of a silent no-op.
                    let result = if resolve_var(&name, &instances, &var_index_by_name).is_some() {
                        if let Ok(mut f) = forces.lock() {
                            f.insert(name.clone(), value);
                        }
                        Ok(value)
                    } else {
                        Err(RuntimeWriteError::UnknownVariable(name))
                    };
                    let _ = ack.send(result);
                }
                RuntimeCommand::UnforceVariable { name, ack } => {
                    if let Ok(mut f) = forces.lock() {
                        f.remove(&name);
                    }
                    let _ = ack.send(Ok(()));
                }
            }
        }

        // Mode check. Paused → skip every unit's cycle (no IO, no run,
        // no output) — the whole plant freezes together. Step{remaining}
        // → execute this tick, decrement at the bottom; when remaining
        // hits 0 transition to Paused. Snapshots still go out while
        // paused (at the regular 10 Hz cadence) so Monitor keeps
        // showing the frozen state.
        let current_mode = mode.lock().map(|m| *m).unwrap_or(RuntimeMode::Running);
        if matches!(current_mode, RuntimeMode::Paused) {
            prev_paused = true;
            if last_snapshot.elapsed() >= SNAPSHOT_PERIOD {
                let now_us = start.elapsed().as_micros() as u64;
                let snapshot = build_snapshot(
                    &runnings,
                    &debug_maps,
                    &string_layouts,
                    &instances,
                    &shared_names,
                    &clocks,
                    now_us,
                );
                let _ = snapshot_tx.send(snapshot);
                last_snapshot = Instant::now();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        }
        if prev_paused {
            // Coming out of a pause (resume or step): re-anchor every
            // unit so the frozen time isn't booked as overruns, and
            // every unit runs on this first tick — a step from pause
            // advances the entire plant by one scan.
            let now = Instant::now();
            for c in clocks.iter_mut() {
                c.next_due = now;
            }
            prev_paused = false;
        }

        // Forces are cloned once per tick (not per unit) so all units
        // see a consistent force set; resolution per name picks the
        // owning unit. Clone under the lock, write outside it.
        let force_set: Vec<(String, i32)> = match forces.lock() {
            Ok(f) => f.iter().map(|(k, &v)| (k.clone(), v)).collect(),
            Err(_) => Vec::new(),
        };

        let tick_now = Instant::now();
        let mut ran_any = false;

        for &i in &exec_order {
            if clocks[i].next_due > tick_now {
                continue;
            }
            ran_any = true;

            // Input phase: bus → this unit's VM variables
            for rm in &unit_inputs[i] {
                let Some(dev) = devices.get_mut(rm.device_index) else {
                    continue;
                };
                match dev.read_channel(&rm.channel).await {
                    Ok(value) => {
                        let _ = if rm.is_lreal {
                            // 64-bit lane: any channel value widens to f64
                            // losslessly; the slot takes the double's bits.
                            runnings[i].write_variable_raw(
                                VarIndex::new(rm.var_index),
                                value.to_f64().to_bits(),
                            )
                        } else {
                            runnings[i].write_variable(
                                VarIndex::new(rm.var_index),
                                value.to_vm_bits(rm.is_real),
                            )
                        };
                    }
                    Err(e) => tracing::debug!(channel = %rm.channel, %e, "input read failed"),
                }
            }

            // Force phase: apply pinned variables that resolve to this
            // unit AFTER its input read (a forced value beats the bus)
            // and BEFORE its run_round (the program sees the forced
            // value).
            for (name, value) in &force_set {
                if let Some((u, idx)) = resolve_var(name, &instances, &var_index_by_name) {
                    if u == i {
                        let _ = runnings[i].write_variable(VarIndex::new(idx), *value);
                    }
                }
            }

            // Run one scan for this unit
            let now_us = start.elapsed().as_micros() as u64;
            if let Err(ctx) = runnings[i].run_round(now_us) {
                tracing::error!(instance = %instances[i], ?ctx.trap, "vm trap during run_round");
                // Record WHY before breaking — the watch write is what
                // wakes the HTTP layer's forwarder, turning a silent halt
                // into a visible fault on /status + SSE.
                fault.send_if_modified(|f| {
                    if f.is_none() {
                        *f = Some(format!("VM trap in {}: {:?}", instances[i], ctx.trap));
                        true
                    } else {
                        false
                    }
                });
                // One faulted unit stops the whole plant — consistent
                // with pause semantics ("the plant freezes together")
                // and the safest default; the wrapper's failsafe pass
                // then zeroes every output.
                vm_fault = true;
                break;
            }
            clocks[i].scan_count += 1;

            // Output phase: this unit's VM variables → bus
            for rm in &unit_outputs[i] {
                let Ok(raw) = runnings[i].read_variable_raw(VarIndex::new(rm.var_index)) else {
                    continue;
                };
                let value = value_for_type(raw, rm.type_tag);
                let Some(dev) = devices.get_mut(rm.device_index) else {
                    continue;
                };
                if let Err(e) = dev.write_channel(&rm.channel, value).await {
                    tracing::debug!(channel = %rm.channel, %e, "output write failed");
                }
            }

            // Cadence advance + per-unit watchdog accounting. An
            // in-time scan clears the unit's overrun streak; an
            // overrun slides its anchor to `now + interval` so we
            // don't burn CPU catching up.
            let interval = clocks[i].interval;
            clocks[i].next_due += interval;
            let after = Instant::now();
            if clocks[i].next_due > after {
                clocks[i].consecutive_overruns = 0;
            } else {
                clocks[i].consecutive_overruns = clocks[i].consecutive_overruns.saturating_add(1);
                if !clocks[i].warned_overrun {
                    let overrun = after - clocks[i].next_due;
                    tracing::warn!(
                        instance = %instances[i],
                        overrun_us = overrun.as_micros() as u64,
                        interval_us = clocks[i].interval.as_micros() as u64,
                        "scan overran its budget — sliding cadence forward and \
                         suppressing further overrun warnings (the trace will \
                         show the drift in scan_count vs wall clock)"
                    );
                    clocks[i].warned_overrun = true;
                }
                if watchdog_armed && clocks[i].consecutive_overruns >= WATCHDOG_OVERRUN_THRESHOLD {
                    tracing::error!(
                        instance = %instances[i],
                        consecutive = clocks[i].consecutive_overruns,
                        threshold = WATCHDOG_OVERRUN_THRESHOLD,
                        interval_us = clocks[i].interval.as_micros() as u64,
                        "watchdog tripped — engaging failsafe (outputs zeroed; \
                         restart the program to re-arm)"
                    );
                    for dev in devices.iter_mut() {
                        if let Err(e) = dev.enter_failsafe().await {
                            tracing::warn!(device = %dev.name(), %e, "watchdog failsafe call failed");
                        }
                    }
                    watchdog_armed = false;
                }
                clocks[i].next_due = after + clocks[i].interval;
            }
        }
        if vm_fault {
            break;
        }

        // If we're stepping, decrement once per tick in which at least
        // one unit ran, and auto-pause at 0. Done AFTER the tick so
        // `step(1)` means "advance by exactly one scheduler tick"
        // (from pause, that's one scan of every unit).
        if ran_any {
            if let RuntimeMode::Step { remaining } = current_mode {
                if let Ok(mut m) = mode.lock() {
                    *m = if remaining <= 1 {
                        RuntimeMode::Paused
                    } else {
                        RuntimeMode::Step {
                            remaining: remaining - 1,
                        }
                    };
                }
            }
        }

        // Snapshot at ~10 Hz (also when no unit was due this tick —
        // idle wake-ups are capped at SNAPSHOT_PERIOD below).
        if last_snapshot.elapsed() >= SNAPSHOT_PERIOD {
            let now_us = start.elapsed().as_micros() as u64;
            let snapshot = build_snapshot(
                &runnings,
                &debug_maps,
                &string_layouts,
                &instances,
                &shared_names,
                &clocks,
                now_us,
            );
            let _ = snapshot_tx.send(snapshot);
            last_snapshot = Instant::now();
        }

        // Persist RETAIN values on a coarse cadence. The window of
        // potential loss on power-cut is bounded by RETAIN_FLUSH_INTERVAL.
        // Writes are atomic (tmp + rename) so a crash during flush
        // can't corrupt the file. Skipped entirely when no retain
        // vars are declared or no path was configured.
        if !retain_entries.is_empty()
            && state_path.is_some()
            && last_retain_flush.elapsed() >= RETAIN_FLUSH_INTERVAL
        {
            let now_us = start.elapsed().as_micros() as u64;
            persist_retain_values(
                state_path.as_deref().unwrap(),
                &retain_entries,
                &runnings,
                now_us,
                max_scan_count(&clocks),
            );
            last_retain_flush = Instant::now();
        }

        // Sleep until the earliest due unit. Capped at SNAPSHOT_PERIOD
        // so stop requests, commands, and paused-state snapshots stay
        // responsive even when every task interval is long.
        let earliest = clocks
            .iter()
            .map(|c| c.next_due)
            .min()
            .expect("at least one unit");
        let now = Instant::now();
        if earliest > now {
            tokio::time::sleep((earliest - now).min(SNAPSHOT_PERIOD)).await;
        }
    }

    // Final RETAIN flush on graceful exit. Captures whatever the
    // last completed scan produced — that's the right "checkpoint"
    // value to reload on next startup.
    if !retain_entries.is_empty() {
        if let Some(path) = state_path.as_deref() {
            let now_us = start.elapsed().as_micros() as u64;
            persist_retain_values(
                path,
                &retain_entries,
                &runnings,
                now_us,
                max_scan_count(&clocks),
            );
            tracing::info!(?path, "final retain flush on stop");
        }
    }

    for running in runnings {
        let _ = running.stop();
    }
}

/// The merged snapshot's scan_count: the max across units (the fastest
/// unit's count — closest analogue of the old single-unit counter).
fn max_scan_count(clocks: &[UnitClock]) -> u64 {
    clocks.iter().map(|c| c.scan_count).max().unwrap_or(0)
}

fn value_for_type(raw: u64, type_tag: u8) -> ChannelValue {
    match type_tag {
        iec_type_tag::BOOL => ChannelValue::Bool(raw != 0),
        iec_type_tag::USINT | iec_type_tag::UINT | iec_type_tag::BYTE | iec_type_tag::WORD => {
            ChannelValue::U16(raw as u16)
        }
        // REAL vars store IEEE-754 bits in the VM cell — decode to a true
        // float so analog outputs keep their fraction on the bus.
        iec_type_tag::REAL => ChannelValue::Real(f32::from_bits(raw as u32)),
        // LREAL: the slot is the double's full bit pattern.
        iec_type_tag::LREAL => ChannelValue::F64(f64::from_bits(raw)),
        _ => ChannelValue::I32(raw as i32),
    }
}

/// Snapshot every RETAIN variable's current VM value (across all units)
/// and atomically write the one merged state file. Keys are already
/// canonical (`instance.variable` when several units run, bare names
/// otherwise) — see the retain_entries construction. Raw slot values
/// verbatim (schema 2): lossless for every IEC type including LREAL /
/// LINT / ULINT / LWORD. Errors are logged but don't crash the scan
/// loop — losing one flush window is acceptable; halting the program
/// is not.
fn persist_retain_values(
    state_path: &std::path::Path,
    retain_entries: &[(usize, String, u16)],
    runnings: &[VmRunning],
    now_us: u64,
    scan_count: u64,
) {
    let mut vars: HashMap<String, u64> = HashMap::with_capacity(retain_entries.len());
    for (unit, key, idx) in retain_entries {
        if let Ok(raw) = runnings[*unit].read_variable_raw(VarIndex::new(*idx)) {
            vars.insert(key.clone(), raw);
        }
    }
    let state = crate::retain::build(vars, now_us, scan_count);
    if let Err(e) = crate::retain::save(state_path, &state) {
        tracing::warn!(?state_path, %e, "retain flush failed");
    }
}

/// Merge every unit's variables into one snapshot. Names declared by
/// more than one unit are disambiguated as `instance.variable`
/// (`shared_names` is precomputed at startup); unique names — and
/// everything in a single-unit run — stay bare so today's projects see
/// no UI churn.
fn build_snapshot(
    runnings: &[VmRunning],
    debug_maps: &[HashMap<u16, VarDebugInfo>],
    string_layouts: &[HashMap<u16, u32>],
    instances: &[String],
    shared_names: &std::collections::HashSet<String>,
    clocks: &[UnitClock],
    now_us: u64,
) -> VarSnapshot {
    let mut vars = Vec::new();
    for (u, running) in runnings.iter().enumerate() {
        let num_vars = running.num_variables();
        vars.reserve(num_vars as usize);
        // STRING bytes live in this unit's data region, not the var slot;
        // read from the SAME unit `u` we're iterating so multi-PROGRAM
        // instances never cross-read each other's strings.
        let data_region = running.data_region();
        // Skip slots that have no debug-map entry (unnamed VM scratch
        // storage for FB internals / non-instantiated POUs) and dedup
        // names that collide within the unit — a unit's source carries
        // every FB/FUNCTION POU, so two POUs declaring the same
        // variable name both get debug entries with that name. We keep
        // the first-seen slot; surfacing the same name twice in the
        // Monitor pane is worse than hiding the inactive duplicate
        // (which is usually idle at zero anyway).
        let mut seen = std::collections::HashSet::<String>::new();
        for i in 0..num_vars {
            let raw = match running.read_variable_raw(VarIndex::new(i)) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let Some(info) = debug_maps[u].get(&i) else {
                continue;
            };
            if !seen.insert(info.name.clone()) {
                continue;
            }
            let name = if shared_names.contains(&info.name) {
                format!("{}.{}", instances[u], info.name)
            } else {
                info.name.clone()
            };
            vars.push(VarValue {
                name,
                type_name: info.type_name.clone(),
                value: format_value(raw, info, string_layouts[u].get(&i).copied(), data_region),
            });
        }
    }
    VarSnapshot {
        timestamp_us: now_us,
        scan_count: max_scan_count(clocks),
        vars,
    }
}

/// Render a variable's slot value. STRING vars don't live in the slot —
/// their bytes are at `data_offset` in the VM's data region with layout
/// `[max_len: u16][cur_len: u16][bytes…]`; we read them out and format
/// as a single-quoted IEC literal. WSTRING isn't yet populated by
/// ironplc's codegen (literal encoding `unreachable!`s on char_width=2),
/// so we surface a placeholder rather than fake content. Everything else
/// delegates to ironplc's standard formatter.
fn format_value(
    raw: u64,
    info: &VarDebugInfo,
    string_offset: Option<u32>,
    data_region: &[u8],
) -> String {
    match info.iec_type_tag {
        iec_type_tag::STRING => {
            match string_offset.and_then(|off| read_string_value(data_region, off)) {
                Some(text) => text,
                // No layout entry, or the offset is bogus — show that we
                // know it's a string but couldn't decode it, not a lie.
                None => "'<invalid>'".into(),
            }
        }
        // ironplc's codegen doesn't yet emit WSTRING data; see
        // `compiler/codegen/src/compile.rs` `unreachable!("WSTRING literal
        // encoding is not yet supported")`. Until upstream lands it, we
        // surface a placeholder so operators know what's missing.
        iec_type_tag::WSTRING => "'<wstring>'".into(),
        _ => format_variable_value(raw, info.iec_type_tag),
    }
}

/// Reads a STRING value at `data_offset` in the VM's data region and
/// renders it as an IEC 61131-3 single-quoted literal (e.g. `'STARTUP'`).
///
/// Wire format: `[max_len: u16][cur_len: u16][cur_len bytes of UTF-8]`,
/// little-endian, as written by ironplc's codegen (see
/// `compiler/codegen/src/compile_setup.rs` `string_region_size`).
/// Non-printable bytes, `$`, and `'` are escaped per IEC string-literal
/// rules so the resulting text is a valid round-trippable literal.
///
/// Returns `None` when the layout offset is past the end of the data
/// region or the recorded `cur_len` would read off the end (e.g. a stale
/// debug section). Bridging a `None` to a placeholder is the caller's job.
fn read_string_value(data_region: &[u8], data_offset: u32) -> Option<String> {
    let off = data_offset as usize;
    if off + STRING_HEADER_BYTES > data_region.len() {
        return None;
    }
    let cur_len = u16::from_le_bytes([data_region[off + 2], data_region[off + 3]]) as usize;
    let start = off + STRING_HEADER_BYTES;
    let end = start + cur_len;
    if end > data_region.len() {
        return None;
    }
    Some(format_iec_string_literal(&data_region[start..end]))
}

/// Renders raw STRING bytes as an IEC 61131-3 single-quoted string literal.
/// Each byte is either passed through as printable ASCII, replaced with one
/// of the named `$`-escapes (`$T`, `$L`, `$P`, `$R`, `$$`, `$'`), or emitted
/// as a `$XX` two-digit hex escape. Mirrors the format ironplc uses in its
/// playground variable dump.
fn format_iec_string_literal(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('\'');
    for &b in bytes {
        match b {
            b'$' => out.push_str("$$"),
            b'\'' => out.push_str("$'"),
            0x09 => out.push_str("$T"),
            0x0A => out.push_str("$L"),
            0x0C => out.push_str("$P"),
            0x0D => out.push_str("$R"),
            0x20..=0x7E => out.push(b as char),
            _ => out.push_str(&format!("${b:02X}")),
        }
    }
    out.push('\'');
    out
}

/// Build a `var_index → data_offset` map from the container's STRING
/// layout sub-table (debug section Tag 4). Empty when no STRING vars
/// were declared.
fn build_string_layout_map(container: &Container) -> HashMap<u16, u32> {
    let mut map = HashMap::new();
    if let Some(debug) = &container.debug_section {
        for entry in &debug.string_layouts {
            map.insert(entry.var_index.raw(), entry.data_offset);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use iocore::IoError;

    /// Records the order of the safety-critical shutdown callbacks so a
    /// test can assert `enter_failsafe` runs before `shutdown` — the
    /// sequence that guarantees zeroed outputs reach the wire before a
    /// device joins its background I/O thread.
    struct MockDevice {
        name: String,
        failsafe_called: Arc<AtomicBool>,
        shutdown_called: Arc<AtomicBool>,
        failsafe_before_shutdown: Arc<AtomicBool>,
        /// Shared health flag so a test can flip transport health
        /// mid-run and watch it surface through the handle.
        healthy: Arc<AtomicBool>,
    }

    impl MockDevice {
        fn named(name: &str) -> (Self, Arc<AtomicBool>) {
            let healthy = Arc::new(AtomicBool::new(true));
            (
                MockDevice {
                    name: name.into(),
                    failsafe_called: Arc::new(AtomicBool::new(false)),
                    shutdown_called: Arc::new(AtomicBool::new(false)),
                    failsafe_before_shutdown: Arc::new(AtomicBool::new(false)),
                    healthy: healthy.clone(),
                },
                healthy,
            )
        }
    }

    #[async_trait::async_trait]
    impl IoDevice for MockDevice {
        fn name(&self) -> &str {
            &self.name
        }
        async fn read_channel(&mut self, _channel: &str) -> Result<ChannelValue, IoError> {
            Ok(ChannelValue::I32(0))
        }
        async fn write_channel(
            &mut self,
            _channel: &str,
            _value: ChannelValue,
        ) -> Result<(), IoError> {
            Ok(())
        }
        async fn enter_failsafe(&mut self) -> Result<(), IoError> {
            self.failsafe_called.store(true, Ordering::Relaxed);
            Ok(())
        }
        async fn shutdown(&mut self) -> Result<(), IoError> {
            if self.failsafe_called.load(Ordering::Relaxed) {
                self.failsafe_before_shutdown.store(true, Ordering::Relaxed);
            }
            self.shutdown_called.store(true, Ordering::Relaxed);
            Ok(())
        }
        fn is_healthy(&self) -> bool {
            self.healthy.load(Ordering::Relaxed)
        }
    }

    fn trivial_container() -> Container {
        // A PROGRAM that starts cleanly; mirrors the lib.rs test source.
        crate::compile(
            "PROGRAM main\n\
                VAR x : INT := 1; END_VAR\n\
                x := x + 1;\n\
            END_PROGRAM",
        )
        .expect("trivial program compiles")
    }

    /// One unit named `"main"` — the conventional instance name for a
    /// single-program run; tests exercising that path use this to stay
    /// representative of production call sites.
    fn single_unit(container: Container, interval_ms: u64) -> ProgramUnit {
        unit("main", container, interval_ms, 1)
    }

    fn unit(instance: &str, container: Container, interval_ms: u64, priority: i32) -> ProgramUnit {
        ProgramUnit {
            instance: instance.into(),
            task_name: "plc_task".into(),
            interval_ms,
            priority,
            container,
            retain_vars: Vec::new(),
        }
    }

    /// Wait for snapshots and return the last one received before
    /// `deadline` elapses. Panics if no snapshot arrived at all.
    async fn last_snapshot_within(
        rx: &mut broadcast::Receiver<VarSnapshot>,
        deadline: Duration,
    ) -> VarSnapshot {
        let until = Instant::now() + deadline;
        let mut last: Option<VarSnapshot> = None;
        loop {
            let now = Instant::now();
            if now >= until {
                break;
            }
            match tokio::time::timeout(until - now, rx.recv()).await {
                Ok(Ok(snap)) => last = Some(snap),
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => break,
            }
        }
        last.expect("scan loop emitted at least one snapshot")
    }

    fn var_value(snap: &VarSnapshot, name: &str) -> String {
        snap.vars
            .iter()
            .find(|v| v.name == name)
            .unwrap_or_else(|| {
                let names: Vec<&str> = snap.vars.iter().map(|v| v.name.as_str()).collect();
                panic!("variable '{name}' not in snapshot; have: {names:?}")
            })
            .value
            .clone()
    }

    #[tokio::test]
    async fn shutdown_runs_failsafe_then_device_shutdown_then_joins() {
        let failsafe_called = Arc::new(AtomicBool::new(false));
        let shutdown_called = Arc::new(AtomicBool::new(false));
        let order_ok = Arc::new(AtomicBool::new(false));
        let dev = MockDevice {
            name: "mock".into(),
            failsafe_called: failsafe_called.clone(),
            shutdown_called: shutdown_called.clone(),
            failsafe_before_shutdown: order_ok.clone(),
            healthy: Arc::new(AtomicBool::new(true)),
        };
        let devices: Vec<Box<dyn IoDevice>> = vec![Box::new(dev)];

        let handle = spawn_units_inner(
            vec![single_unit(trivial_container(), 10)],
            DeviceSource::Prebuilt(devices),
            Vec::new(),
            None,
        );
        // Let a few scans run so we're stopping a live loop, not a cold one.
        tokio::time::sleep(Duration::from_millis(40)).await;

        // The whole drain must finish well within the supervisor budget;
        // the timeout also keeps a regression from hanging the suite.
        tokio::time::timeout(Duration::from_secs(5), handle.shutdown())
            .await
            .expect("shutdown joined the scan thread within 5s");

        assert!(
            failsafe_called.load(Ordering::Relaxed),
            "failsafe must run on a clean stop"
        );
        assert!(
            shutdown_called.load(Ordering::Relaxed),
            "device shutdown must run on a clean stop"
        );
        assert!(
            order_ok.load(Ordering::Relaxed),
            "failsafe must precede device shutdown so zeroed outputs flush before the join"
        );
    }

    /// A device that fails its initial connect must come back through the
    /// background reconnect worker: /discover flips to connected, health
    /// flips true, and — the part an operator actually cares about — the
    /// mapping that was parked at startup binds, so the device's register
    /// value flows into the PLC variable mid-run without a restart.
    #[tokio::test]
    async fn background_reconnect_adopts_failed_device_and_binds_mappings() {
        use project::{
            ModbusChannel, ModbusChannelKind, ModbusConfig, ModbusDataType, ModbusTcpParams,
            ModbusTransport, ModbusWordOrder,
        };

        // Reserve a port, then close the listener → initial connect refused.
        let reserved = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        let spec = DeviceSpec {
            name: "pm".into(),
            config: ProtocolConfig::Modbus(ModbusConfig {
                transport: ModbusTransport::Tcp(ModbusTcpParams {
                    host: addr.ip().to_string(),
                    port: addr.port(),
                }),
                slave_id: 1,
                poll_interval_ms: 20,
                timeout_ms: Some(200),
                reconnect_backoff_ms: None,
                channels: vec![ModbusChannel {
                    name: "hr0".into(),
                    kind: ModbusChannelKind::HoldingRegister,
                    address: 0,
                    data_type: ModbusDataType::U16,
                    word_order: ModbusWordOrder::HiLo,
                }],
            }),
        };
        let mapping = Mapping {
            application: "main".into(),
            variable: "x".into(),
            direction: Direction::Input,
            device: "pm".into(),
            channel: "hr0".into(),
        };
        // `y := x` so the input mapping's value is observable unmodified.
        let container = crate::compile(
            "PROGRAM main\n\
                VAR x : INT; y : INT; END_VAR\n\
                y := x;\n\
            END_PROGRAM",
        )
        .expect("program compiles");

        let handle = spawn_units_inner(
            vec![single_unit(container, 10)],
            DeviceSource::Specs(vec![spec]),
            vec![mapping],
            None,
        );

        // Startup outcome: configured but down, and visibly so.
        let deadline = Instant::now() + Duration::from_secs(5);
        while handle.device_reports().is_empty() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let reports = handle.device_reports();
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].connected, "no endpoint yet — connect must fail");
        assert!(
            !handle.device_health()[0].healthy,
            "failed connect must surface as unhealthy"
        );

        // Bring a real slave up on the reserved port with hr[0] = 42.
        let slave = iomap_modbus::DemoSlave::new();
        slave.holding_registers().lock().expect("hr")[0] = 42;
        let server = tokio::spawn(iomap_modbus::run_demo_slave(addr, slave));

        // Worker backoff is 1 s, 2 s, 4 s… — well within this budget.
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if handle.device_reports()[0].connected && handle.device_health()[0].healthy {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            handle.device_reports()[0].connected,
            "background reconnect must flip the /discover report"
        );
        assert!(
            handle.device_health()[0].healthy,
            "adopted device must go healthy"
        );

        // The parked mapping must now be live: register 42 → x → y.
        let mut rx = handle.subscribe();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = false;
        while Instant::now() < deadline && !seen {
            let snap = last_snapshot_within(&mut rx, Duration::from_millis(300)).await;
            seen = var_value(&snap, "x") == "42";
        }
        assert!(
            seen,
            "register value must flow through the late-bound mapping"
        );

        tokio::time::timeout(Duration::from_secs(5), handle.shutdown())
            .await
            .expect("shutdown");
        server.abort();
    }

    /// The handle must mirror a device's live transport health: `true`
    /// while the adapter reports healthy, `false` within a scan round of
    /// the adapter flipping (cable pulled / poll loop failing), and back —
    /// this is the surface /health and /status serve to operators, whose
    /// only other signal is "variable values stopped changing".
    #[tokio::test]
    async fn device_health_mirrors_adapter_flag() {
        let (dev, healthy) = MockDevice::named("bus0");
        let handle = spawn_units_inner(
            vec![single_unit(trivial_container(), 5)],
            DeviceSource::Prebuilt(vec![Box::new(dev)]),
            Vec::new(),
            None,
        );

        // Wait for the connect pass to seed the health list.
        let deadline = Instant::now() + Duration::from_secs(2);
        while handle.device_health().is_empty() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let seeded = handle.device_health();
        assert_eq!(seeded.len(), 1, "one configured device");
        assert_eq!(seeded[0].name, "bus0");
        assert!(seeded[0].healthy, "healthy adapter seeds healthy");

        // Flip the adapter's flag; the scan loop refreshes each round.
        healthy.store(false, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(2);
        while handle.device_health()[0].healthy && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            !handle.device_health()[0].healthy,
            "unhealthy adapter must surface through the handle"
        );

        // And recovery propagates too.
        healthy.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !handle.device_health()[0].healthy && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(handle.device_health()[0].healthy, "recovery must surface");

        tokio::time::timeout(Duration::from_secs(5), handle.shutdown())
            .await
            .expect("shutdown");
    }

    /// F64 device channel → LREAL var → F64 device channel, verifying
    /// the 64-bit lane end to end. The probe value carries more
    /// precision than an f32 can hold, so any accidental trip through
    /// the 32-bit path fails the exact-equality assert.
    struct LrealLoopDevice {
        name: String,
        input_value: f64,
        written: Arc<std::sync::Mutex<Option<f64>>>,
    }

    #[async_trait::async_trait]
    impl IoDevice for LrealLoopDevice {
        fn name(&self) -> &str {
            &self.name
        }
        async fn read_channel(&mut self, _channel: &str) -> Result<ChannelValue, IoError> {
            Ok(ChannelValue::F64(self.input_value))
        }
        async fn write_channel(
            &mut self,
            _channel: &str,
            value: ChannelValue,
        ) -> Result<(), IoError> {
            if let ChannelValue::F64(v) = value {
                *self.written.lock().unwrap() = Some(v);
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn lreal_mapping_round_trips_full_double_precision() {
        // Needs 64-bit mantissa: f32 would collapse the tail digits.
        let probe = 1234.5678901234567_f64;
        assert_ne!(
            probe as f32 as f64, probe,
            "probe must exceed f32 precision"
        );

        let container = crate::compile(
            "PROGRAM main\n\
                VAR lr_in : LREAL; lr_out : LREAL; END_VAR\n\
                lr_out := lr_in;\n\
            END_PROGRAM",
        )
        .expect("LREAL program compiles");

        let written = Arc::new(std::sync::Mutex::new(None));
        let dev = LrealLoopDevice {
            name: "mock".into(),
            input_value: probe,
            written: written.clone(),
        };
        let devices: Vec<Box<dyn IoDevice>> = vec![Box::new(dev)];
        let mappings = vec![
            project::Mapping {
                application: "main".into(),
                variable: "lr_in".into(),
                direction: project::Direction::Input,
                device: "mock".into(),
                channel: "ain".into(),
            },
            project::Mapping {
                application: "main".into(),
                variable: "lr_out".into(),
                direction: project::Direction::Output,
                device: "mock".into(),
                channel: "aout".into(),
            },
        ];

        let handle = spawn_units_inner(
            vec![single_unit(container, 10)],
            DeviceSource::Prebuilt(devices),
            mappings,
            None,
        );
        tokio::time::sleep(Duration::from_millis(80)).await;
        tokio::time::timeout(Duration::from_secs(5), handle.shutdown())
            .await
            .expect("shutdown joins");

        let got = written.lock().unwrap().expect("output channel was written");
        assert_eq!(
            got, probe,
            "LREAL must cross device→VM→device without 32-bit truncation"
        );
    }

    #[tokio::test]
    async fn shutdown_is_idempotent() {
        let (dev, _healthy) = MockDevice::named("mock");
        let devices: Vec<Box<dyn IoDevice>> = vec![Box::new(dev)];
        let handle = spawn_units_inner(
            vec![single_unit(trivial_container(), 10)],
            DeviceSource::Prebuilt(devices),
            Vec::new(),
            None,
        );

        handle.shutdown().await;
        // Second call has no thread left to join; must return immediately.
        tokio::time::timeout(Duration::from_secs(1), handle.shutdown())
            .await
            .expect("second shutdown returns immediately");
    }

    /// Two units at different intervals: the 10 ms unit must rack up
    /// notably more scans than the 50 ms unit over the same window.
    /// Per-unit scan counts are observed via each program's own
    /// increment-per-scan counter variable (distinct names → bare names
    /// in the merged snapshot).
    #[tokio::test]
    async fn two_units_scan_at_their_own_intervals() {
        let fast = crate::compile(
            "PROGRAM pfast\n\
                VAR fast_count : DINT; END_VAR\n\
                fast_count := fast_count + 1;\n\
            END_PROGRAM",
        )
        .expect("fast program compiles");
        let slow = crate::compile(
            "PROGRAM pslow\n\
                VAR slow_count : DINT; END_VAR\n\
                slow_count := slow_count + 1;\n\
            END_PROGRAM",
        )
        .expect("slow program compiles");

        let handle = spawn_units_inner(
            vec![
                unit("fast_inst", fast, 10, 1),
                unit("slow_inst", slow, 50, 2),
            ],
            DeviceSource::Prebuilt(Vec::new()),
            Vec::new(),
            None,
        );
        let mut rx = handle.subscribe();
        let snap = last_snapshot_within(&mut rx, Duration::from_millis(250)).await;
        tokio::time::timeout(Duration::from_secs(5), handle.shutdown())
            .await
            .expect("shutdown joins");

        let fast_scans: i64 = var_value(&snap, "fast_count")
            .parse()
            .expect("fast_count is numeric");
        let slow_scans: i64 = var_value(&snap, "slow_count")
            .parse()
            .expect("slow_count is numeric");
        assert!(slow_scans >= 1, "slow unit must run at all: {slow_scans}");
        // Nominal ratio is 5 (10 ms vs 50 ms); CI scheduling jitter
        // eats some of it, but anything ≤ 2 means per-unit cadence is
        // broken (both units sharing one clock would give ratio 1).
        assert!(
            fast_scans > 2 * slow_scans,
            "fast unit must scan >2x more often: fast={fast_scans} slow={slow_scans}"
        );
    }

    /// Constant-value input device: every `read_channel` yields the same
    /// i32 so routing tests can tell which unit's variable was fed.
    struct ConstInputDevice {
        name: String,
        value: i32,
    }

    #[async_trait::async_trait]
    impl IoDevice for ConstInputDevice {
        fn name(&self) -> &str {
            &self.name
        }
        async fn read_channel(&mut self, _channel: &str) -> Result<ChannelValue, IoError> {
            Ok(ChannelValue::I32(self.value))
        }
        async fn write_channel(
            &mut self,
            _channel: &str,
            _value: ChannelValue,
        ) -> Result<(), IoError> {
            Ok(())
        }
    }

    /// `Mapping.application` routes a device channel to ONE unit: both
    /// units declare a variable `x`, the mapping names instance "a", so
    /// only a's x sees the bus value while b's stays at its initial 0.
    /// The colliding name is read back instance-prefixed from the
    /// merged snapshot.
    #[tokio::test]
    async fn mapping_routes_to_the_named_instance_only() {
        let prog = |name: &str| {
            crate::compile(&format!(
                "PROGRAM {name}\n\
                    VAR x : INT; mirror : INT; END_VAR\n\
                    mirror := x;\n\
                END_PROGRAM"
            ))
            .expect("program compiles")
        };

        let dev = ConstInputDevice {
            name: "mock".into(),
            value: 42,
        };
        let devices: Vec<Box<dyn IoDevice>> = vec![Box::new(dev)];
        let mappings = vec![project::Mapping {
            application: "a".into(),
            variable: "x".into(),
            direction: project::Direction::Input,
            device: "mock".into(),
            channel: "ain".into(),
        }];

        let handle = spawn_units_inner(
            vec![unit("a", prog("pa"), 10, 1), unit("b", prog("pb"), 10, 1)],
            DeviceSource::Prebuilt(devices),
            mappings,
            None,
        );
        let mut rx = handle.subscribe();
        let snap = last_snapshot_within(&mut rx, Duration::from_millis(150)).await;
        tokio::time::timeout(Duration::from_secs(5), handle.shutdown())
            .await
            .expect("shutdown joins");

        assert_eq!(
            var_value(&snap, "a.x"),
            "42",
            "instance a's x must receive the device input"
        );
        assert_eq!(
            var_value(&snap, "b.x"),
            "0",
            "instance b's x must NOT receive the device input"
        );
        // The program logic of the routed unit also saw the value.
        assert_eq!(var_value(&snap, "a.mirror"), "42");
        assert_eq!(var_value(&snap, "b.mirror"), "0");
    }

    /// A STRING var initialised to a literal must surface as the quoted
    /// IEC value in the snapshot. Exercises the data-region read path
    /// (`[max_len][cur_len][bytes…]` at the layout-table offset) end to
    /// end through the live scan loop, not just the formatter — and the
    /// per-unit `string_layouts[u]` / `runnings[u].data_region()` wiring.
    #[tokio::test]
    async fn snapshot_renders_string_var_as_quoted_iec_literal() {
        let prog = crate::compile(
            "PROGRAM string_smoke\n\
                VAR operating_mode : STRING := 'STARTUP'; END_VAR\n\
            END_PROGRAM",
        )
        .expect("string program compiles");

        let handle = spawn_units_inner(
            vec![single_unit(prog, 10)],
            DeviceSource::Prebuilt(Vec::new()),
            Vec::new(),
            None,
        );
        let mut rx = handle.subscribe();
        let snap = last_snapshot_within(&mut rx, Duration::from_millis(250)).await;
        tokio::time::timeout(Duration::from_secs(5), handle.shutdown())
            .await
            .expect("shutdown joins");

        assert_eq!(var_value(&snap, "operating_mode"), "'STARTUP'");
    }
}
