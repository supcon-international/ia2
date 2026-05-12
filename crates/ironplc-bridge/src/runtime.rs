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
    WriteVariable {
        name: String,
        value: i32,
        ack: tokio::sync::oneshot::Sender<Result<i32, RuntimeWriteError>>,
    },
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
}

pub fn spawn(
    container: Container,
    devices: Vec<DeviceSpec>,
    mappings: Vec<Mapping>,
) -> ProgramHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let (snapshot_tx, _) = broadcast::channel(64);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();

    let stop_clone = stop.clone();
    let snapshot_tx_clone = snapshot_tx.clone();

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
        ));
    });

    ProgramHandle {
        stop,
        snapshot_tx,
        cmd_tx,
    }
}

async fn run_loop_async(
    container: Container,
    device_specs: Vec<DeviceSpec>,
    mappings: Vec<Mapping>,
    stop: Arc<AtomicBool>,
    snapshot_tx: broadcast::Sender<VarSnapshot>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<RuntimeCommand>,
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

        // Drain any pending out-of-band commands (variable writes etc.)
        // Non-blocking — apply what's ready, leave the rest for the next
        // round. Drops the ack channel on failure to signal disconnect.
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
            }
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
