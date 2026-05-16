//! Bridges the ironplc VM scan loop to:
//!  - `tokio::sync::broadcast` for streaming `VarSnapshot`s to subscribers.
//!  - `iocore::IoDevice` adapters for reading inputs before `run_round` and
//!    writing outputs after.
//!
//! The scan thread is a dedicated `std::thread` that hosts a single-thread
//! tokio runtime; everything bus-related runs inside it. ironplc's
//! `VmRunning::run_round` itself is sync.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ironplc_container::Container;
use ironplc_container::VarIndex;
use ironplc_container::debug_format::{VarDebugInfo, build_var_debug_map, format_variable_value};
use ironplc_container::debug_section::iec_type_tag;
use ironplc_vm::{Vm, VmBuffers, VmRunning};
use iocore::{ChannelValue, IoDevice};
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
    pub async fn write_variable(
        &self,
        name: &str,
        value: i32,
    ) -> Result<i32, RuntimeWriteError> {
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
    pub async fn force_variable(
        &self,
        name: &str,
        value: i32,
    ) -> Result<i32, RuntimeWriteError> {
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
    pub async fn unforce_variable(
        &self,
        name: &str,
    ) -> Result<(), RuntimeWriteError> {
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
                let mut out: Vec<(String, i32)> =
                    m.iter().map(|(k, &v)| (k.clone(), v)).collect();
                out.sort_by(|a, b| a.0.cmp(&b.0));
                out
            })
            .unwrap_or_default()
    }

    /// Read the current execution mode (Running / Paused / Step{N}).
    pub fn mode(&self) -> RuntimeMode {
        self.mode
            .lock()
            .map(|m| *m)
            .unwrap_or(RuntimeMode::Running)
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
}

pub fn spawn(
    container: Container,
    devices: Vec<DeviceSpec>,
    mappings: Vec<Mapping>,
) -> ProgramHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let (snapshot_tx, _) = broadcast::channel(64);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let mode = Arc::new(std::sync::Mutex::new(RuntimeMode::Running));
    let forces = Arc::new(std::sync::Mutex::new(HashMap::<String, i32>::new()));

    let stop_clone = stop.clone();
    let snapshot_tx_clone = snapshot_tx.clone();
    let mode_clone = mode.clone();
    let forces_clone = forces.clone();

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
        rt.block_on(run_loop_async(
            container,
            devices,
            mappings,
            stop_clone,
            snapshot_tx_clone,
            cmd_rx,
            mode_clone,
            forces_clone,
        ));
    });

    ProgramHandle {
        stop,
        snapshot_tx,
        cmd_tx,
        mode,
        forces,
    }
}

async fn run_loop_async(
    container: Container,
    device_specs: Vec<DeviceSpec>,
    mappings: Vec<Mapping>,
    stop: Arc<AtomicBool>,
    snapshot_tx: broadcast::Sender<VarSnapshot>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<RuntimeCommand>,
    mode: Arc<std::sync::Mutex<RuntimeMode>>,
    forces: Arc<std::sync::Mutex<HashMap<String, i32>>>,
) {
    // ---- Connect devices (skip the ones that fail rather than abort) ----
    let mut devices: Vec<Box<dyn IoDevice>> = Vec::with_capacity(device_specs.len());
    for spec in device_specs {
        match &spec.config {
            ProtocolConfig::Modbus(cfg) => {
                match iomap_modbus::ModbusDevice::connect(spec.name.clone(), cfg).await {
                    Ok(d) => {
                        tracing::info!(name = %spec.name, host = %cfg.host, port = cfg.port, "modbus connected");
                        devices.push(Box::new(d));
                    }
                    Err(e) => tracing::warn!(name = %spec.name, %e, "modbus connect failed"),
                }
            }
            ProtocolConfig::Ethercat(cfg) => {
                match iomap_ethercat::EthercatDevice::connect(spec.name.clone(), cfg).await {
                    Ok(d) => {
                        tracing::info!(name = %spec.name, nic = %cfg.nic, "ethercat connected");
                        devices.push(Box::new(d));
                    }
                    Err(e) => tracing::warn!(name = %spec.name, %e, "ethercat connect failed"),
                }
            }
        }
    }

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
        let type_tag = debug_map.get(&var_index).map(|d| d.iec_type_tag).unwrap_or(0);
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
        "scan loop ready"
    );

    // ---- Scan loop ----
    let start = Instant::now();
    let mut last_snapshot = Instant::now() - Duration::from_secs(1);
    let mut scan_count: u64 = 0;

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
                        Some(idx) => match running
                            .write_variable(VarIndex::new(idx), value)
                        {
                            Ok(()) => Ok(value),
                            Err(trap) => {
                                Err(RuntimeWriteError::Vm(format!("{trap:?}")))
                            }
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

        // Sleep until next task is due (or briefly yield for freewheeling)
        if let Some(due_us) = running.next_due_us() {
            let now_us = start.elapsed().as_micros() as u64;
            let sleep_us = due_us.saturating_sub(now_us);
            if sleep_us > 0 {
                tokio::time::sleep(Duration::from_micros(sleep_us)).await;
            }
        } else {
            tokio::time::sleep(Duration::from_millis(1)).await;
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
