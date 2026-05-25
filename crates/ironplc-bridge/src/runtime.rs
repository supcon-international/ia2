//! Bridges the ironplc VM scan loop to:
//!  - `tokio::sync::broadcast` for streaming `VarSnapshot`s to subscribers.
//!  - `iocore::IoDevice` adapters for reading inputs before `run_round` and
//!    writing outputs after.
//!
//! The scan thread is a dedicated `std::thread` that hosts a single-thread
//! tokio runtime; everything bus-related runs inside it. ironplc's
//! `VmRunning::run_round` itself is sync.

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
}

impl ProgramHandle {
    /// Cooperative stop. The scan loop checks the flag at the top of each
    /// round; expect a few extra rounds before it actually exits.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
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

/// All knobs the run path can set when starting a scan loop. The old
/// `spawn` / `spawn_with_interval` entry points stay as thin wrappers
/// for code that doesn't care about retain persistence.
#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    /// Cycle period in milliseconds. `0` (or omitted) falls back to
    /// `DEFAULT_SCAN_INTERVAL_MS`.
    pub scan_interval_ms: u64,
    /// IEC variable names declared `VAR RETAIN` — extracted from the
    /// AST by `ironplc_bridge::compile_with_metadata`. Empty means "no
    /// retain persistence".
    pub retain_vars: Vec<String>,
    /// Where to load/save retain values. `None` disables persistence
    /// (in-memory only) — useful for the IDE's ephemeral demo runs.
    /// The runtime crate points this at `<install_dir>/state/retain.json`
    /// so values survive systemd restarts and redeploys.
    pub state_path: Option<PathBuf>,
}

pub fn spawn(
    container: Container,
    devices: Vec<DeviceSpec>,
    mappings: Vec<Mapping>,
) -> ProgramHandle {
    spawn_with_interval(container, devices, mappings, DEFAULT_SCAN_INTERVAL_MS)
}

/// Compatibility wrapper — preserves the old API for callers that
/// don't need retain options. New code should call `spawn_with_options`
/// directly.
pub fn spawn_with_interval(
    container: Container,
    device_specs: Vec<DeviceSpec>,
    mappings: Vec<Mapping>,
    scan_interval_ms: u64,
) -> ProgramHandle {
    spawn_with_options(
        container,
        device_specs,
        mappings,
        SpawnOptions {
            scan_interval_ms,
            ..Default::default()
        },
    )
}

/// Like `spawn`, but throttles the scan loop to
/// `options.scan_interval_ms` regardless of what (if anything) the
/// compiled CONFIGURATION requested, AND persists RETAIN variables
/// across runs when `options.state_path` is set.
///
/// Why we throttle in the bridge rather than letting the VM
/// scheduler do it: as of the currently-vendored ironplc, codegen
/// does NOT populate `container.task_table`. The VM scheduler
/// therefore sees zero cyclic tasks → `next_due_us()` returns
/// `None` → the scan loop falls through to a 1 ms minimum sleep
/// → ~700 scans/s. That breaks every time-sensitive demo
/// (PID tunings, SFC transition timings, TON expirations). Until
/// upstream wires CONFIGURATION → task_table, this bridge-level
/// throttle is the source of truth for "the scan period the user
/// asked for in tasks.toml."
pub fn spawn_with_options(
    container: Container,
    device_specs: Vec<DeviceSpec>,
    mappings: Vec<Mapping>,
    options: SpawnOptions,
) -> ProgramHandle {
    let SpawnOptions {
        scan_interval_ms,
        retain_vars,
        state_path,
    } = options;
    let scan_interval_ms = if scan_interval_ms == 0 {
        DEFAULT_SCAN_INTERVAL_MS
    } else {
        scan_interval_ms
    };
    let stop = Arc::new(AtomicBool::new(false));
    let (snapshot_tx, _) = broadcast::channel(64);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let mode = Arc::new(std::sync::Mutex::new(RuntimeMode::Running));
    let forces = Arc::new(std::sync::Mutex::new(HashMap::<String, i32>::new()));
    let device_reports = Arc::new(std::sync::Mutex::new(Vec::<DeviceReport>::new()));

    let stop_clone = stop.clone();
    let snapshot_tx_clone = snapshot_tx.clone();
    let mode_clone = mode.clone();
    let forces_clone = forces.clone();
    let device_reports_clone = device_reports.clone();
    let scan_interval = Duration::from_millis(scan_interval_ms.max(1));

    std::thread::spawn(move || {
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
            let (mut devices, reports) = connect_devices(device_specs).await;
            if let Ok(mut slot) = device_reports_clone.lock() {
                *slot = reports;
            }

            // Wrap the scan loop in catch_unwind so a panic in the VM
            // glue / iomap / snapshot fan-out doesn't skip failsafe.
            // `AssertUnwindSafe` is needed because `&mut Vec<...>` is
            // not auto-UnwindSafe; we accept that risk because the
            // only failure mode here is "panic in async code", and
            // we're about to discard `running` anyway.
            use futures_util::FutureExt;
            use std::panic::AssertUnwindSafe;
            let result = AssertUnwindSafe(run_loop_async(
                container,
                &mut devices,
                mappings,
                stop_clone,
                snapshot_tx_clone,
                cmd_rx,
                mode_clone,
                forces_clone,
                scan_interval,
                retain_vars,
                state_path,
            ))
            .catch_unwind()
            .await;

            // Always-run failsafe before the thread dies. Drive every
            // device's outputs to zero so a hung / panicked / stopped
            // program doesn't leave actuators energized.
            let dev_count = devices.len();
            for dev in devices.iter_mut() {
                if let Err(e) = dev.enter_failsafe().await {
                    tracing::warn!(device = %dev.name(), %e, "failsafe call failed");
                }
            }
            // Give real EtherCAT one extra cycle to actually push the
            // zeros onto the bus before its worker thread exits via
            // Drop. Conservative 50 ms covers cycle_us up to 25 ms; we
            // never run faster than that in practice and the latency
            // is only paid once on shutdown.
            tokio::time::sleep(Duration::from_millis(50)).await;
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
) -> (Vec<Box<dyn IoDevice>>, Vec<DeviceReport>) {
    let mut devices: Vec<Box<dyn IoDevice>> = Vec::with_capacity(device_specs.len());
    let mut reports: Vec<DeviceReport> = Vec::with_capacity(device_specs.len());
    for spec in device_specs {
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
                        devices.push(Box::new(d));
                        reports.push(DeviceReport {
                            name: spec.name.clone(),
                            protocol: "modbus".into(),
                            connected: true,
                            error: None,
                            slaves: Vec::new(),
                        });
                    }
                    Err(e) => {
                        tracing::warn!(name = %spec.name, %e, "modbus connect failed");
                        reports.push(DeviceReport {
                            name: spec.name.clone(),
                            protocol: "modbus".into(),
                            connected: false,
                            error: Some(e.to_string()),
                            slaves: Vec::new(),
                        });
                    }
                }
            }
            ProtocolConfig::Ethercat(cfg) => {
                match iomap_ethercat::EthercatDevice::connect(spec.name.clone(), cfg).await {
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
                        devices.push(Box::new(d));
                        reports.push(DeviceReport {
                            name: spec.name.clone(),
                            protocol: "ethercat".into(),
                            connected: true,
                            error: None,
                            slaves,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(name = %spec.name, %e, "ethercat connect failed");
                        reports.push(DeviceReport {
                            name: spec.name.clone(),
                            protocol: "ethercat".into(),
                            connected: false,
                            error: Some(e.to_string()),
                            slaves: Vec::new(),
                        });
                    }
                }
            }
        }
    }
    (devices, reports)
}

/// Trip the watchdog after this many consecutive scan deadline overruns.
/// Each overrun means the scan body didn't finish within `scan_interval`.
/// 5 in a row → the simulation has lost real-time guarantees; engage
/// failsafe and don't re-arm until the program is restarted.
const WATCHDOG_OVERRUN_THRESHOLD: u32 = 5;

#[allow(clippy::too_many_arguments)]
async fn run_loop_async(
    container: Container,
    // Devices are borrowed so the outer wrapper retains ownership for
    // its always-run failsafe pass (see `spawn_with_interval`'s async
    // block).
    devices: &mut Vec<Box<dyn IoDevice>>,
    mappings: Vec<Mapping>,
    stop: Arc<AtomicBool>,
    snapshot_tx: broadcast::Sender<VarSnapshot>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<RuntimeCommand>,
    mode: Arc<std::sync::Mutex<RuntimeMode>>,
    forces: Arc<std::sync::Mutex<HashMap<String, i32>>>,
    scan_interval: Duration,
    // RETAIN persistence — names extracted from VAR RETAIN blocks,
    // plus the disk path to load/save them from. Both empty / None
    // disables persistence with no behavioral change vs the
    // pre-retain code path.
    retain_vars: Vec<String>,
    state_path: Option<PathBuf>,
) {
    // ---- Start the VM ----
    let mut bufs = VmBuffers::from_container(&container);
    let mut running = match Vm::new().load(&container, &mut bufs).start() {
        Ok(r) => r,
        Err(ctx) => {
            tracing::error!(?ctx.trap, "vm failed to start");
            return;
        }
    };

    let debug_map = build_var_debug_map(&container);

    // ---- Resolve mappings into index pairs ----
    let var_index_by_name: HashMap<String, u16> = debug_map
        .iter()
        .map(|(idx, info)| (info.name.clone(), *idx))
        .collect();
    let device_index_by_name: HashMap<String, usize> = devices
        .iter()
        .enumerate()
        .map(|(i, d)| (d.name().to_string(), i))
        .collect();

    struct ResolvedMapping {
        device_index: usize,
        channel: String,
        var_index: u16,
        type_tag: u8,
    }

    let mut inputs: Vec<ResolvedMapping> = Vec::new();
    let mut outputs: Vec<ResolvedMapping> = Vec::new();
    for m in mappings {
        let Some(&device_index) = device_index_by_name.get(&m.device) else {
            tracing::warn!(device = %m.device, "mapping references unknown device, skipping");
            continue;
        };
        let Some(&var_index) = var_index_by_name.get(&m.variable) else {
            tracing::warn!(var = %m.variable, "mapping references unknown variable, skipping");
            continue;
        };
        let type_tag = debug_map
            .get(&var_index)
            .map(|d| d.iec_type_tag)
            .unwrap_or(0);
        let rm = ResolvedMapping {
            device_index,
            channel: m.channel.clone(),
            var_index,
            type_tag,
        };
        match m.direction {
            Direction::Input => inputs.push(rm),
            Direction::Output => outputs.push(rm),
        }
    }
    tracing::info!(
        inputs = inputs.len(),
        outputs = outputs.len(),
        devices = devices.len(),
        retain_vars = retain_vars.len(),
        state_path = ?state_path,
        "scan loop ready"
    );

    // ---- Resolve RETAIN names → var indices (drop unknowns loudly) ----
    let retain_indices: Vec<(String, u16)> = retain_vars
        .iter()
        .filter_map(|name| match var_index_by_name.get(name) {
            Some(&idx) => Some((name.clone(), idx)),
            None => {
                // Possible if the user removed a RETAIN var from
                // source between runs but the state file still
                // references it. Skip silently in the loud-warn map
                // step; the next save will rewrite the file without
                // the stale entry.
                tracing::warn!(var = %name, "retain var not in debug map; skipping");
                None
            }
        })
        .collect();

    // ---- Restore RETAIN values from disk before scanning starts ----
    if !retain_indices.is_empty() {
        if let Some(path) = state_path.as_ref() {
            match retain::load(path) {
                Ok(Some(state)) => {
                    let mut restored = 0;
                    for (name, idx) in &retain_indices {
                        if let Some(&value) = state.vars.get(name) {
                            if running.write_variable(VarIndex::new(*idx), value).is_ok() {
                                restored += 1;
                            } else {
                                tracing::warn!(var = %name, "restore write trapped; skipping");
                            }
                        }
                    }
                    tracing::info!(
                        restored,
                        total = retain_indices.len(),
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
    let mut scan_count: u64 = 0;
    // Cadence anchor: when the *next* scan should start. Each cycle
    // sleeps until this instant, then advances it by one
    // `scan_interval`. If a scan overruns its budget (the simulation
    // does heavy work this round), `next_scan_at` slides forward by
    // `now + scan_interval` so we don't burn CPU catching up; the
    // overrun is logged via a one-shot warning.
    let mut next_scan_at = Instant::now() + scan_interval;
    let mut warned_overrun = false;
    // Watchdog: counts consecutive overruns. Reset to 0 on any in-
    // time scan. When it reaches WATCHDOG_OVERRUN_THRESHOLD we fire
    // failsafe once and disarm — keeps the loop running but never
    // re-fires; the user has to restart the program after a watchdog
    // trip (industrial convention).
    let mut consecutive_overruns: u32 = 0;
    let mut watchdog_armed = true;

    loop {
        if stop.load(Ordering::Relaxed) {
            running.request_stop();
        }
        if running.stop_requested() {
            break;
        }

        // Drain any pending out-of-band commands (variable writes,
        // forces, unforces). Done EVERY iteration including when
        // paused — that's the whole point of pausing, you want to
        // poke values without scanning.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                RuntimeCommand::WriteVariable { name, value, ack } => {
                    let result = match var_index_by_name.get(&name).copied() {
                        Some(idx) => match running.write_variable(VarIndex::new(idx), value) {
                            Ok(()) => Ok(value),
                            Err(trap) => Err(RuntimeWriteError::Vm(format!("{trap:?}"))),
                        },
                        None => Err(RuntimeWriteError::UnknownVariable(name)),
                    };
                    let _ = ack.send(result);
                }
                RuntimeCommand::ForceVariable { name, value, ack } => {
                    // Reject unknown names early so the caller gets
                    // immediate feedback instead of a silent no-op.
                    let result = if var_index_by_name.contains_key(&name) {
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

        // Mode check. Paused → skip the whole cycle (no IO, no run,
        // no output). Step{remaining} → execute this cycle, decrement
        // at the bottom; when remaining hits 0 transition to Paused.
        // Snapshot still gets emitted while paused (at the regular
        // 10 Hz cadence) so Monitor keeps showing the frozen state.
        let current_mode = mode.lock().map(|m| *m).unwrap_or(RuntimeMode::Running);
        if matches!(current_mode, RuntimeMode::Paused) {
            // Still emit periodic snapshots while paused so Monitor
            // stays alive and operators can see frozen values without
            // a refresh. Use the elapsed-since-start clock for the
            // snapshot timestamp (same as the running path).
            if last_snapshot.elapsed() >= Duration::from_millis(100) {
                let now_us = start.elapsed().as_micros() as u64;
                let snapshot = build_snapshot(&running, &debug_map, now_us, scan_count);
                let _ = snapshot_tx.send(snapshot);
                last_snapshot = Instant::now();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        }

        // Input phase: bus → VM variables
        for rm in &inputs {
            let Some(dev) = devices.get_mut(rm.device_index) else {
                continue;
            };
            match dev.read_channel(&rm.channel).await {
                Ok(value) => {
                    let _ = running.write_variable(VarIndex::new(rm.var_index), value.to_i32());
                }
                Err(e) => tracing::debug!(channel = %rm.channel, %e, "input read failed"),
            }
        }

        // Force phase: apply each pinned variable AFTER input read so
        // a forced value beats the bus, and BEFORE run_round so the
        // program sees the forced value (program-side writes during
        // the round may transiently differ, but the next cycle re-
        // applies the force). Clone the snapshot under the lock so
        // we don't hold the mutex across .write_variable.
        let snapshot: Vec<(String, i32)> = match forces.lock() {
            Ok(f) => f.iter().map(|(k, &v)| (k.clone(), v)).collect(),
            Err(_) => Vec::new(),
        };
        for (name, value) in snapshot {
            if let Some(&idx) = var_index_by_name.get(&name) {
                let _ = running.write_variable(VarIndex::new(idx), value);
            }
        }

        // Run one scheduling round
        let now_us = start.elapsed().as_micros() as u64;
        if let Err(ctx) = running.run_round(now_us) {
            tracing::error!(?ctx.trap, "vm trap during run_round");
            break;
        }
        scan_count += 1;

        // Output phase: VM variables → bus
        for rm in &outputs {
            let Ok(raw) = running.read_variable_raw(VarIndex::new(rm.var_index)) else {
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

        // If we're stepping, decrement and auto-pause once we hit 0.
        // Done AFTER the round so `step(1)` means "advance by exactly
        // one full cycle".
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

        // Snapshot at ~10 Hz
        if last_snapshot.elapsed() >= Duration::from_millis(100) {
            let snapshot = build_snapshot(&running, &debug_map, now_us, scan_count);
            let _ = snapshot_tx.send(snapshot);
            last_snapshot = Instant::now();
        }

        // Persist RETAIN values on a coarse cadence. The window of
        // potential loss on power-cut is bounded by RETAIN_FLUSH_INTERVAL.
        // Writes are atomic (tmp + rename) so a crash during flush
        // can't corrupt the file. Skipped entirely when no retain
        // vars are declared or no path was configured.
        if !retain_indices.is_empty()
            && state_path.is_some()
            && last_retain_flush.elapsed() >= RETAIN_FLUSH_INTERVAL
        {
            persist_retain_values(
                state_path.as_deref().unwrap(),
                &retain_indices,
                &running,
                now_us,
                scan_count,
            );
            last_retain_flush = Instant::now();
        }

        // Sleep until the next scan deadline. The VM's `next_due_us`
        // would also work in theory, but as of the vendored ironplc
        // codegen doesn't populate the container's task_table from
        // the CONFIGURATION block — so `next_due_us` returns None
        // and the loop free-runs at the underlying tokio scheduler's
        // resolution (~1 ms). The bridge owns the cadence here,
        // sourced from `tasks.toml` via spawn_with_interval.
        let now = Instant::now();
        if now < next_scan_at {
            // Reset the watchdog counter on any in-time scan — a single
            // recovery clears prior near-misses.
            consecutive_overruns = 0;
            tokio::time::sleep(next_scan_at - now).await;
            next_scan_at += scan_interval;
        } else {
            consecutive_overruns = consecutive_overruns.saturating_add(1);
            if !warned_overrun {
                let overrun = now - next_scan_at;
                tracing::warn!(
                    overrun_us = overrun.as_micros() as u64,
                    interval_us = scan_interval.as_micros() as u64,
                    "scan overran its budget — sliding cadence forward and \
                     suppressing further overrun warnings (the trace will \
                     show the drift in scan_count vs wall clock)"
                );
                warned_overrun = true;
            }
            // Watchdog trip: N consecutive misses → engage failsafe
            // exactly once. We keep scanning afterward (don't break)
            // so the operator can see live state via the snapshot
            // stream, but outputs stay safe until the program is
            // restarted.
            if watchdog_armed && consecutive_overruns >= WATCHDOG_OVERRUN_THRESHOLD {
                tracing::error!(
                    consecutive = consecutive_overruns,
                    threshold = WATCHDOG_OVERRUN_THRESHOLD,
                    interval_us = scan_interval.as_micros() as u64,
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
            next_scan_at = now + scan_interval;
        }
    }

    // Final RETAIN flush on graceful exit. Captures whatever the
    // last completed scan produced — that's the right "checkpoint"
    // value to reload on next startup.
    if !retain_indices.is_empty() {
        if let Some(path) = state_path.as_deref() {
            let now_us = start.elapsed().as_micros() as u64;
            persist_retain_values(path, &retain_indices, &running, now_us, scan_count);
            tracing::info!(?path, "final retain flush on stop");
        }
    }

    let _ = running.stop();
}

fn value_for_type(raw: u64, type_tag: u8) -> ChannelValue {
    match type_tag {
        iec_type_tag::BOOL => ChannelValue::Bool(raw != 0),
        iec_type_tag::USINT | iec_type_tag::UINT | iec_type_tag::BYTE | iec_type_tag::WORD => {
            ChannelValue::U16(raw as u16)
        }
        _ => ChannelValue::I32(raw as i32),
    }
}

/// Snapshot every RETAIN variable's current VM value and atomically
/// write the state file. Errors are logged but don't crash the scan
/// loop — losing one flush window is acceptable; halting the program
/// is not. The VM's `read_variable_raw` yields u64; we down-cast to
/// i32 because that's what `write_variable` (used at restore) accepts.
/// LREAL / LINT / LWORD lose their upper 32 bits — documented as a
/// known limitation pending an ironplc upstream change.
fn persist_retain_values(
    state_path: &std::path::Path,
    retain_indices: &[(String, u16)],
    running: &VmRunning,
    now_us: u64,
    scan_count: u64,
) {
    let mut vars: HashMap<String, i32> = HashMap::with_capacity(retain_indices.len());
    for (name, idx) in retain_indices {
        if let Ok(raw) = running.read_variable_raw(VarIndex::new(*idx)) {
            vars.insert(name.clone(), raw as i32);
        }
    }
    let state = crate::retain::build(vars, now_us, scan_count);
    if let Err(e) = crate::retain::save(state_path, &state) {
        tracing::warn!(?state_path, %e, "retain flush failed");
    }
}

fn build_snapshot(
    running: &VmRunning,
    debug_map: &HashMap<u16, VarDebugInfo>,
    now_us: u64,
    scan_count: u64,
) -> VarSnapshot {
    let num_vars = running.num_variables();
    let mut vars = Vec::with_capacity(num_vars as usize);
    // Skip slots that have no debug-map entry (unnamed VM scratch storage
    // for FB internals / non-instantiated POUs) and dedup names that
    // collide across POU types — `compile_project` concatenates every
    // POU's source, so two POUs declaring the same variable name both
    // get debug entries with that name. We keep the first-seen slot;
    // surfacing the same name twice in the Monitor pane is worse than
    // hiding the inactive duplicate (which is usually idle at zero anyway).
    let mut seen = std::collections::HashSet::<String>::new();
    for i in 0..num_vars {
        let raw = match running.read_variable_raw(VarIndex::new(i)) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let Some(info) = debug_map.get(&i) else {
            continue;
        };
        if !seen.insert(info.name.clone()) {
            continue;
        }
        vars.push(VarValue {
            name: info.name.clone(),
            type_name: info.type_name.clone(),
            value: format_variable_value(raw, info.iec_type_tag),
        });
    }
    VarSnapshot {
        timestamp_us: now_us,
        scan_count,
        vars,
    }
}
