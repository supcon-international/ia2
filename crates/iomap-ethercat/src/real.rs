//! Real `ethercrab::MainDevice` driver. Selected when `EthercatConfig.nic`
//! is anything other than `"_sim"`.
//!
//! Architecture:
//!
//! - `connect` spawns a **dedicated OS thread** that owns ethercrab. We
//!   need our own thread because ethercrab's `tx_rx_task` is built on
//!   `async-io`, not tokio, and conflicting reactors in the same thread
//!   would deadlock. The thread runs `smol::block_on`, which drives
//!   `async-io` natively.
//!
//! - The thread:
//!   1. `Box::leak`s a `PduStorage` (gives `&'static`; required by
//!      `try_split`). One leaked storage per `connect` — fine, devices
//!      don't churn at runtime.
//!   2. Builds the `MainDevice`, spawns `tx_rx_task` as a detached smol
//!      task, walks the bus with `init_single_group`, transitions to OP.
//!   3. Reports back through a `tokio::sync::oneshot` so the connect()
//!      future awaits an "actually live" signal (or an init error).
//!   4. Enters the cyclic loop, exiting when the shutdown flag flips.
//!
//! - **PDI mirror** (`Arc<Mutex<PdiMirror>>`): the cyclic task is the
//!   sole owner of the `SubDeviceGroup`. Reader/writer paths
//!   (`read_channel` / `write_channel`) never touch ethercrab — they
//!   only lock the mirror briefly. Each cycle:
//!     - Pre-cycle: copy `mirror.outputs[slave_index]` → group's per-slave
//!       output PDI bytes.
//!     - `group.tx_rx(...).await` — actual fieldbus exchange.
//!     - Post-cycle: copy group's per-slave input PDI bytes →
//!       `mirror.inputs[slave_index]`.
//!
//!   The result: the program's scan loop sees consistent inputs and the
//!   bus sees consistent outputs each round, without ethercrab ever
//!   being touched from a tokio task.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::{mpsc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ethercrab::std::ethercat_now;
use ethercrab::subdevice_group::DcConfiguration;
use ethercrab::{DcSync, MainDevice, MainDeviceConfig, PduStorage, Timeouts};
use iocore::{ChannelValue, HealthTracker, HealthTransition, IoDevice, IoError};
use project::{
    EthercatChannel, EthercatConfig, EthercatDcSync, EthercatPdoDirection, EthercatSlave,
};

use crate::bits;
use crate::validate;
use crate::SlaveDiscovery;

/// Consecutive cyclic `tx_rx` failures before the device is flagged
/// unhealthy (one ERROR log per outage, not one per cycle — at a 1 ms
/// cycle that distinction matters).
const UNHEALTHY_AFTER_TX_ERRORS: u32 = 10;

// Storage sizing — picked to comfortably cover a typical edge configuration
// (an EK1100-class coupler + EL modules). Sized for plant-scale buses,
// not just demo benches: a 1000-point project (AI230/AO54/DI480/DO270)
// needs ~660 B of PDI and tens of modular subdevices, so 128 subdevices
// / 4096 B PDI leaves comfortable headroom. The cost is static memory
// only (PduStorage is Box::leaked per connect): 64 frames x ~1100 B is
// about 70 KB — irrelevant on an edge box — and one 4 KiB PDI cycle
// splits across ~4 PDUs per direction, which 64 in-flight frames cover
// several times over. MAX_SUBDEVICES must be a power of 2 > 1;
// MAX_PDU_DATA at ~1100 tracks the Ethernet frame limit.
const MAX_SUBDEVICES: usize = 128;
const MAX_PDU_DATA: usize = PduStorage::element_size(1100);
const MAX_FRAMES: usize = 64;
const PDI_LEN: usize = 4096;

type Storage = PduStorage<MAX_FRAMES, MAX_PDU_DATA>;

// These three macros share the per-cycle / discovery logic between the DC
// (`Sync0`) and free-run (`Off`) paths in `smol_main`. They're macros, not
// generic fns, to sidestep the `SubDeviceGroup` typestate generics (HasDc
// vs NoDc are distinct types) without duplicating ~40 lines twice.

/// Walk the OP group, size the PDI mirror, log + return the discovered
/// SubDevices.
macro_rules! capture_discovery {
    ($group:expr, $maindevice:expr, $pdi:expr) => {{
        let mut discovered: Vec<SlaveDiscovery> = Vec::new();
        {
            let mut mirror = $pdi.lock().expect("pdi mirror poisoned");
            for (offset_idx, sd) in $group.iter(&$maindevice).enumerate() {
                let io = sd.io_raw();
                let in_len = io.inputs().len();
                let out_len = io.outputs().len();
                let idx = offset_idx as u16;
                mirror.inputs.insert(idx, vec![0u8; in_len]);
                mirror.outputs.insert(idx, vec![0u8; out_len]);
                discovered.push(SlaveDiscovery {
                    index: idx,
                    name: sd.name().to_string(),
                    input_bytes: in_len as u16,
                    output_bytes: out_len as u16,
                    vendor_id: sd.identity().vendor_id,
                    product_id: sd.identity().product_id,
                });
            }
        }
        for sd in &discovered {
            tracing::info!(
                slave = sd.index,
                sd_name = %sd.name,
                vendor = format!("{:#010x}", sd.vendor_id),
                product = format!("{:#010x}", sd.product_id),
                in_bytes = sd.input_bytes,
                out_bytes = sd.output_bytes,
                "discovered subdevice"
            );
        }
        discovered
    }};
}

/// Pre-cycle: copy our owned output bytes onto the bus surface.
macro_rules! copy_outputs_to_bus {
    ($group:expr, $maindevice:expr, $pdi:expr) => {{
        let mirror = $pdi.lock().expect("pdi mirror poisoned");
        for (offset_idx, sd) in $group.iter(&$maindevice).enumerate() {
            let idx = offset_idx as u16;
            if let Some(src) = mirror.outputs.get(&idx) {
                let mut out = sd.outputs_raw_mut();
                let n = out.len().min(src.len());
                out[..n].copy_from_slice(&src[..n]);
            }
        }
    }};
}

/// In-cycle gear tick: for every configured gear axis, read the follower's
/// statusword + actual (and the master axis's actual) from the *input*
/// surface of the previous exchange, run the engine, and overwrite the
/// follower's target_position bytes on the *output* surface. Runs after
/// `copy_outputs_to_bus!` so the engine — not the PLC mirror — owns those
/// bytes on the wire.
macro_rules! gear_tick {
    ($group:expr, $maindevice:expr, $engines:expr, $bus_ok:expr) => {{
        for eng in $engines.iter_mut() {
            let mut master_actual: Option<i32> = None;
            if let crate::gear::MasterSrc::Axis {
                slave_index,
                offset,
            } = eng.master
            {
                for (i, sd) in $group.iter(&$maindevice).enumerate() {
                    if i as u16 == slave_index {
                        master_actual = crate::gear::read_i32(&sd.inputs_raw()[..], offset);
                        break;
                    }
                }
            }
            for (i, sd) in $group.iter(&$maindevice).enumerate() {
                if i as u16 == eng.follower_index {
                    let (sw, actual) = {
                        let inp = sd.inputs_raw();
                        (
                            crate::gear::read_u16(&inp[..], eng.status_off).unwrap_or(0),
                            crate::gear::read_i32(&inp[..], eng.actual_off).unwrap_or(0),
                        )
                    };
                    let target = eng.tick(sw, actual, master_actual, $bus_ok);
                    let mut out = sd.outputs_raw_mut();
                    crate::gear::write_i32(&mut out[..], eng.target_off, target);
                    break;
                }
            }
        }
    }};
}

/// Post-cycle: snapshot inputs back into our mirror.
macro_rules! copy_inputs_from_bus {
    ($group:expr, $maindevice:expr, $pdi:expr) => {{
        let mut mirror = $pdi.lock().expect("pdi mirror poisoned");
        for (offset_idx, sd) in $group.iter(&$maindevice).enumerate() {
            let idx = offset_idx as u16;
            let inputs = sd.inputs_raw();
            if let Some(dst) = mirror.inputs.get_mut(&idx) {
                let n = inputs.len().min(dst.len());
                dst[..n].copy_from_slice(&inputs[..n]);
            }
        }
    }};
}

/// Per-slave byte mirrors of the PDI. Indexed by `EthercatSlave.index`
/// (which matches the auto-incremented bus position ethercrab assigns).
///
/// Inputs are written by the cyclic task post-tx_rx; reads from
/// `read_channel` extract bits out of these buffers.
///
/// Outputs are written by `write_channel`; the cyclic task copies them
/// onto the bus pre-tx_rx. Reads of RxPDO channels echo back from this
/// same buffer (matches sim mode and is useful for "did my write take?"
/// debugging in the UI).
#[derive(Default, Debug)]
struct PdiMirror {
    inputs: HashMap<u16, Vec<u8>>,
    outputs: HashMap<u16, Vec<u8>>,
}

/// Bounded wait for the cyclic worker thread to stop on graceful
/// shutdown. The loop observes `shutdown` within ~one cycle (plus, at
/// worst, one ethercrab PDU timeout on a wedged bus), then flushes a
/// final zeroed frame and exits — comfortably inside this budget. Capped
/// so a hung bus can't push the whole shutdown past systemd's kill
/// timeout: if we blow it, we abandon the thread and let process
/// teardown reap it.
const WORKER_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

pub struct RealEthercat {
    name: String,
    channels: HashMap<String, EthercatChannel>,
    /// Slow-plane routing for in-cycle gear parameter channels; consulted
    /// by read/write_channel before the PDI channel lookup.
    gear_routing: crate::gear::GearRouting,
    pdi: Arc<Mutex<PdiMirror>>,
    shutdown: Arc<AtomicBool>,
    /// Flipped by the worker (via a drop guard) on every `smol_main`
    /// exit path, so `shutdown` can join with a bound instead of risking
    /// a wait on a thread that's already gone.
    stopped: Arc<AtomicBool>,
    /// Mirrored from the cyclic worker's `HealthTracker`: `false` after
    /// `UNHEALTHY_AFTER_TX_ERRORS` consecutive `tx_rx` failures, `true`
    /// again on the first successful exchange.
    healthy: Arc<AtomicBool>,
    /// Subdevices found during the bus walk at connect (for `/discover`).
    discovered: Vec<SlaveDiscovery>,
    // The cyclic worker. Joined by `shutdown` on a clean stop (so the
    // final zeroed frame is guaranteed on the wire); on `Drop` it's only
    // signalled, not joined — drop can run mid-teardown and must not block.
    _thread: Option<thread::JoinHandle<()>>,
}

/// The `Arc` handles `connect` shares with the cyclic worker thread,
/// bundled so `smol_main` keeps a readable signature.
struct WorkerShared {
    pdi: Arc<Mutex<PdiMirror>>,
    shutdown: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    healthy: Arc<AtomicBool>,
    /// In-cycle gear engines (one per configured [[gear]] axis), owned by
    /// the cyclic worker and ticked between the mirror copy and tx_rx.
    engines: Vec<crate::gear::GearEngine>,
}

/// Wait up to `timeout` for the worker to flag `stopped`, then join it.
/// Returns `true` if the thread was joined, `false` if it didn't stop in
/// time (caller abandons the handle; the process exit reaps the thread).
/// Sync + bounded so it can run on a blocking pool without ever holding
/// the shutdown past the service supervisor's deadline.
fn join_worker(
    stopped: &AtomicBool,
    handle: Option<thread::JoinHandle<()>>,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while !stopped.load(Ordering::Relaxed) {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
    if let Some(handle) = handle {
        let _ = handle.join();
    }
    true
}

/// Whether the bus-side init succeeded. Sent back over the oneshot.
#[derive(Debug)]
enum InitResult {
    Ok { discovered: Vec<SlaveDiscovery> },
    Err(String),
}

impl RealEthercat {
    pub async fn connect(name: String, config: &EthercatConfig) -> Result<Self, IoError> {
        // ESI-modular real-bus cyclic bring-up (master-programmed
        // SyncManager/FMMU + logical-RW exchange, bypassing the auto
        // PDO-assignment path) is validated against the physical coupler
        // before it ships — we don't run un-hardware-verified EtherCAT
        // cyclic I/O on a live control bus. The ESI parse/assembly and the
        // offline channel authoring (`cs device esi-assemble`) work today;
        // author + verify the program in sim (`nic: "_sim"`) meanwhile.
        if matches!(config.bringup, project::EthercatBringup::EsiModular { .. }) {
            return Err(IoError::Connect(
                "ESI-modular real-bus bring-up is not yet wired (issue #11): assemble channels \
                 with `cs device esi-assemble` and validate the program in sim (nic \"_sim\") — \
                 the real-bus SM/FMMU/LRW path lands after per-coupler hardware validation"
                    .into(),
            ));
        }

        // Validate channels up front (same shape as sim).
        let known_slaves: std::collections::HashSet<u16> =
            config.slaves.iter().map(|s| s.index).collect();
        let mut seen_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for ch in &config.channels {
            if !seen_names.insert(ch.name.as_str()) {
                return Err(IoError::Connect(format!(
                    "duplicate channel name '{}'",
                    ch.name
                )));
            }
            if !known_slaves.is_empty() && !known_slaves.contains(&ch.slave_index) {
                return Err(IoError::Connect(format!(
                    "channel '{}' references unknown slave_index={}",
                    ch.name, ch.slave_index
                )));
            }
        }

        // Shape problems (zero-length / misaligned entries) are knowable
        // before touching the bus — reject them before spawning anything.
        validate::validate_channel_shapes(&config.channels).map_err(IoError::Connect)?;
        validate::validate_init_sdo(&config.slaves).map_err(IoError::Connect)?;

        // Gear axes: channel names must be unique (across gears and within
        // each gear) and must not shadow PDO channels — the routing check in
        // read/write_channel runs first and would silently eat a collision.
        crate::gear::validate_channels(&config.gear, &seen_names).map_err(IoError::Connect)?;
        // Every referenced slave must exist in the declared topology.
        for g in &config.gear {
            if !known_slaves.is_empty() && !known_slaves.contains(&g.slave_index) {
                return Err(IoError::Connect(format!(
                    "gear follower references unknown slave_index={}",
                    g.slave_index
                )));
            }
            if let project::GearMaster::Axis { slave_index, .. } = g.master {
                if !known_slaves.is_empty() && !known_slaves.contains(&slave_index) {
                    return Err(IoError::Connect(format!(
                        "gear master references unknown slave_index={slave_index}"
                    )));
                }
            }
        }
        let (engines, gear_routing) = crate::gear::build(&config.gear);

        let channels: HashMap<String, EthercatChannel> = config
            .channels
            .iter()
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        let pdi = Arc::new(Mutex::new(PdiMirror::default()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let healthy = Arc::new(AtomicBool::new(true));
        let (init_tx, init_rx) = mpsc::sync_channel::<InitResult>(1);

        let nic = config.nic.clone();
        let cycle_us = config.cycle_us.max(100); // hard floor: don't melt the CPU
        let dc_sync = config.dc_sync;
        let dc_static_sync_iterations = config.dc_static_sync_iterations;
        let slaves = config.slaves.clone();
        let shared = WorkerShared {
            pdi: pdi.clone(),
            shutdown: shutdown.clone(),
            stopped: stopped.clone(),
            healthy: healthy.clone(),
            engines,
        };
        let thread_name = format!("ec-{name}");

        let thread = thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                smol_main(
                    &nic,
                    cycle_us,
                    dc_sync,
                    dc_static_sync_iterations,
                    &slaves,
                    shared,
                    init_tx,
                )
            })
            .map_err(|e| IoError::Connect(format!("spawn ethercat thread: {e}")))?;

        // Wait for the worker to report success or failure. The init walk
        // is bounded (timeouts inside ethercrab); a generous wait here is
        // OK because connect() runs at startup, not in the hot path.
        let init = tokio::task::spawn_blocking(move || {
            init_rx
                .recv_timeout(Duration::from_secs(15))
                .map_err(|e| format!("init handshake timed out: {e}"))
        })
        .await
        .map_err(|e| IoError::Connect(format!("init join: {e}")))?
        .map_err(IoError::Connect)?;

        match init {
            InitResult::Ok { discovered } => {
                // The bus is walked and the cyclic worker is live — now
                // hold the discovered topology against the configuration.
                // Identity first (wrong/missing module), then PDI ranges
                // (channel windows that the real PDI cannot serve). Fail
                // the connect with everything found at once, instead of
                // letting each problem surface later as per-cycle
                // Transport errors or, worse, bits driven on the wrong
                // module.
                let mut problems: Vec<String> = Vec::new();
                if let Err(m) = validate::validate_identities(&config.slaves, &discovered) {
                    problems.push(m);
                }
                if let Err(m) = validate::validate_pdi_ranges(&config.channels, &discovered) {
                    problems.push(m);
                }
                for g in &config.gear {
                    let find = |idx: u16| discovered.iter().find(|d| d.index == idx);
                    match find(g.slave_index) {
                        Some(d) => {
                            if g.target_pos_offset as usize + 4 > d.output_bytes as usize {
                                problems.push(format!(
                                    "gear follower slave {} target_pos_offset {}+4 exceeds output PDI ({} B)",
                                    g.slave_index, g.target_pos_offset, d.output_bytes
                                ));
                            }
                            if g.actual_pos_offset as usize + 4 > d.input_bytes as usize
                                || g.status_word_offset as usize + 2 > d.input_bytes as usize
                            {
                                problems.push(format!(
                                    "gear follower slave {} actual/status offsets exceed input PDI ({} B)",
                                    g.slave_index, d.input_bytes
                                ));
                            }
                        }
                        None => problems.push(format!(
                            "gear follower slave_index {} not on the discovered bus",
                            g.slave_index
                        )),
                    }
                    if let project::GearMaster::Axis {
                        slave_index,
                        actual_pos_offset,
                    } = g.master
                    {
                        match find(slave_index) {
                            Some(d) if (actual_pos_offset as usize + 4) <= d.input_bytes as usize => {}
                            Some(d) => problems.push(format!(
                                "gear master slave {} actual_pos_offset {}+4 exceeds input PDI ({} B)",
                                slave_index, actual_pos_offset, d.input_bytes
                            )),
                            None => problems.push(format!(
                                "gear master slave_index {slave_index} not on the discovered bus"
                            )),
                        }
                    }
                }
                if !problems.is_empty() {
                    // A failed connect must not leak a live cyclic thread
                    // driving the bus: signal it down and join (bounded).
                    shutdown.store(true, Ordering::Relaxed);
                    let stopped_for_join = stopped.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        join_worker(&stopped_for_join, Some(thread), WORKER_JOIN_TIMEOUT)
                    })
                    .await;
                    return Err(IoError::Connect(format!(
                        "ethercat bus validation failed: {}",
                        problems.join("; ")
                    )));
                }

                tracing::info!(
                    name = %name,
                    nic = %config.nic,
                    cycle_us,
                    discovered = discovered.len(),
                    "ethercat device live"
                );
                Ok(Self {
                    name,
                    channels,
                    gear_routing,
                    pdi,
                    shutdown,
                    stopped,
                    healthy,
                    discovered,
                    _thread: Some(thread),
                })
            }
            InitResult::Err(e) => Err(IoError::Connect(format!("ethercat init: {e}"))),
        }
    }

    /// Subdevices found during the bus walk at connect.
    pub fn discovered(&self) -> Vec<SlaveDiscovery> {
        self.discovered.clone()
    }
}

impl Drop for RealEthercat {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // Don't join here — drop can run mid-teardown and must never block
        // the runtime. The worker exits on the next cycle tick. A clean
        // stop goes through `shutdown()` instead, which DOES join (after
        // failsafe) so the zeroed frame is guaranteed on the wire.
    }
}

#[async_trait]
impl IoDevice for RealEthercat {
    fn name(&self) -> &str {
        &self.name
    }

    /// `false` once the cyclic worker has seen `UNHEALTHY_AFTER_TX_ERRORS`
    /// consecutive `tx_rx` failures (cable pulled, slave dropped off,
    /// watchdog tripped); `true` again after the first good exchange.
    /// While unhealthy the PDI mirror serves last-known inputs.
    fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    async fn read_channel(&mut self, channel: &str) -> Result<ChannelValue, IoError> {
        // Gear parameter / feedback channels live in the lock-free shared
        // params, not the PDI.
        if let Some(v) = self.gear_routing.read(channel) {
            return Ok(v);
        }
        let meta = self
            .channels
            .get(channel)
            .ok_or_else(|| IoError::UnknownChannel(channel.into()))?
            .clone();
        let pdi = self.pdi.lock().expect("pdi mirror poisoned");
        let map = match meta.direction {
            EthercatPdoDirection::TxPdo => &pdi.inputs,
            EthercatPdoDirection::RxPdo => &pdi.outputs,
        };
        let buffer = map.get(&meta.slave_index).ok_or_else(|| {
            IoError::Transport(format!(
                "slave_index={} not present in discovered bus",
                meta.slave_index
            ))
        })?;
        bits::read_value(
            buffer,
            meta.pdi_byte_offset as usize,
            meta.pdi_bit_offset,
            meta.bit_length,
            meta.data_type,
        )
    }

    async fn write_channel(&mut self, channel: &str, value: ChannelValue) -> Result<(), IoError> {
        // Gear parameter channels route into the lock-free shared params
        // the cyclic loop reads each tick — never into PDI bytes.
        if let Some(res) = self.gear_routing.write(channel, &value) {
            return res;
        }
        let meta = self
            .channels
            .get(channel)
            .ok_or_else(|| IoError::UnknownChannel(channel.into()))?
            .clone();
        if meta.direction == EthercatPdoDirection::TxPdo {
            return Err(IoError::TypeMismatch {
                channel: channel.into(),
                value,
            });
        }
        let mut pdi = self.pdi.lock().expect("pdi mirror poisoned");
        let buffer = pdi.outputs.get_mut(&meta.slave_index).ok_or_else(|| {
            IoError::Transport(format!(
                "slave_index={} not present in discovered bus",
                meta.slave_index
            ))
        })?;
        bits::write_value(
            buffer,
            meta.pdi_byte_offset as usize,
            meta.pdi_bit_offset,
            meta.bit_length,
            meta.data_type,
            value,
        )
    }

    /// Zero the entire output PDI mirror for every discovered slave.
    /// The next cyclic tick on the worker thread copies these zeros
    /// onto the bus surface and `tx_rx`'s them out — at default cycle
    /// times that's a millisecond or two. Wait a couple of cycle
    /// periods after returning if you need to guarantee propagation
    /// before exiting (the bridge does this).
    async fn enter_failsafe(&mut self) -> Result<(), IoError> {
        // Gear axes first: drop every engage request so the engines fall
        // back to shadow/hold instead of re-driving targets over the
        // zeroed mirror below.
        self.gear_routing.disengage_all();
        let mut pdi = self.pdi.lock().expect("pdi mirror poisoned");
        for buf in pdi.outputs.values_mut() {
            buf.fill(0);
        }
        tracing::info!(device = %self.name, "ethercat output PDI zeroed for failsafe");
        Ok(())
    }

    /// Graceful teardown: signal the cyclic worker to stop and JOIN it.
    /// By this point `enter_failsafe` has zeroed the output mirror, so the
    /// worker's final flush (one last `tx_rx`) puts controlword = 0 on the
    /// wire before it exits. Joining guarantees that frame was sent before
    /// the master goes away — the whole point of the in-runtime failsafe.
    /// Bounded so a wedged bus can't stall the process past its kill
    /// timeout. The join runs on a blocking thread so we don't park the
    /// async executor on a std thread join.
    async fn shutdown(&mut self) -> Result<(), IoError> {
        self.shutdown.store(true, Ordering::Relaxed);
        let handle = self._thread.take();
        let stopped = self.stopped.clone();
        let joined =
            tokio::task::spawn_blocking(move || join_worker(&stopped, handle, WORKER_JOIN_TIMEOUT))
                .await
                .unwrap_or(false);
        if joined {
            tracing::info!(device = %self.name, "ethercat cyclic worker joined (final failsafe frame flushed)");
        } else {
            tracing::warn!(
                device = %self.name,
                timeout_s = WORKER_JOIN_TIMEOUT.as_secs(),
                "ethercat cyclic worker did not stop in time; abandoning thread (process teardown will reap it)"
            );
        }
        Ok(())
    }
}

/// Health bookkeeping for one failed cyclic exchange. Exactly one ERROR
/// per outage (when the threshold is crossed); WARN before that, DEBUG
/// for the repeats while already unhealthy (a dead bus at a 1 ms cycle
/// would otherwise log a thousand lines a second).
fn note_txrx_failure<E: std::fmt::Debug>(health: &mut HealthTracker, error: &E) {
    match health.record_failure() {
        HealthTransition::BecameUnhealthy => {
            tracing::error!(
                ?error,
                consecutive_failures = health.consecutive_failures(),
                "ethercat unhealthy: cyclic tx_rx failing; inputs frozen at last-known values"
            );
        }
        _ if health.is_healthy() => tracing::warn!(?error, "ethercat tx_rx failed"),
        _ => tracing::debug!(?error, "ethercat tx_rx still failing"),
    }
}

/// The entire ethercrab session lives inside this function — bus walk,
/// state-machine transition to OP, and the cyclic exchange loop. Runs on
/// its own thread under `smol::block_on` to drive `async-io`.
#[allow(clippy::too_many_arguments)]
fn smol_main(
    nic: &str,
    cycle_us: u32,
    dc_sync: EthercatDcSync,
    dc_static_sync_iterations: u32,
    slaves: &[EthercatSlave],
    shared: WorkerShared,
    init_tx: mpsc::SyncSender<InitResult>,
) {
    let WorkerShared {
        pdi,
        shutdown,
        stopped,
        healthy,
        mut engines,
    } = shared;
    // Flip `stopped` on EVERY exit path (init failure or loop end) via a
    // drop guard, so a bounded join never waits on a thread that's already
    // gone. Created first thing so even the early `try_split` error returns
    // flag completion.
    struct DoneGuard(Arc<AtomicBool>);
    impl Drop for DoneGuard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let _done = DoneGuard(stopped);

    // One leaked PduStorage per connect. ethercrab requires `&'static`
    // for `try_split`; Box::leak is the textbook idiom. Bounded by the
    // number of EtherCAT devices the user creates per process lifetime,
    // which is small.
    let storage: &'static Storage = Box::leak(Box::new(Storage::new()));
    let (tx, rx, pdu_loop) = match storage.try_split() {
        Ok(triple) => triple,
        Err(_) => {
            let _ = init_tx.send(InitResult::Err(
                "PduStorage already split (one EtherCAT device per process)".into(),
            ));
            return;
        }
    };

    let maindevice = Arc::new(MainDevice::new(
        pdu_loop,
        Timeouts {
            wait_loop_delay: Duration::from_millis(2),
            mailbox_response: Duration::from_millis(1000),
            ..Default::default()
        },
        // Configurable because the right value depends on the bus: 0
        // (our default) skips the init-time FRMW burst entirely — on a
        // short bus / non-RT host any one of those frames timing out
        // aborts init with Timeout(Pdu) (ethercrab's own default of
        // 10_000 is what made single-SubDevice bring-up fail). Longer DC
        // buses that care about clock convergence at OP-entry can raise
        // it from the device config.
        MainDeviceConfig {
            dc_static_sync_iterations,
            ..MainDeviceConfig::default()
        },
    ));

    let nic_owned = nic.to_string();
    smol::block_on(async move {
        // Background: TX/RX socket pump. Detached — it lives until the
        // PduStorage is dropped (which never happens, since we leaked it).
        let tx_rx = match ethercrab::std::tx_rx_task(&nic_owned, tx, rx) {
            Ok(fut) => fut,
            Err(e) => {
                let _ = init_tx.send(InitResult::Err(format!(
                    "tx_rx_task on {nic_owned}: {e} (need CAP_NET_RAW + real NIC)"
                )));
                return;
            }
        };
        let _tx_rx_handle = smol::spawn(async move {
            if let Err(e) = tx_rx.await {
                tracing::error!(?e, "ethercat tx_rx task exited");
            }
        });

        // Walk the bus and assign each SubDevice an auto-increment address.
        let mut group = match maindevice
            .init_single_group::<MAX_SUBDEVICES, PDI_LEN>(ethercat_now)
            .await
        {
            Ok(g) => g,
            Err(e) => {
                let _ = init_tx.send(InitResult::Err(format!("init_single_group: {e:?}")));
                return;
            }
        };

        // Early bus census: log every SubDevice's identity *now*, in PRE-OP,
        // before any init_sdo / PDO / OP step that a non-matching device can
        // abort (e.g. a coupler with no 0x6060, or one that rejects the CoE
        // 0x1600 PDO-assign). This makes `cs edge scan` work as a pure
        // discovery probe against unknown hardware: you always see what's on
        // the wire, even when the configured device can't reach OP.
        for (pos, sd) in group.iter(&maindevice).enumerate() {
            let id = sd.identity();
            tracing::info!(
                slave = pos,
                sd_name = %sd.name(),
                vendor = format!("{:#010x}", id.vendor_id),
                product = format!("{:#010x}", id.product_id),
                revision = format!("{:#010x}", id.revision),
                serial = format!("{:#010x}", id.serial),
                "bus census (PRE-OP)"
            );
        }

        // Per-SubDevice startup SDO writes (PRE-OP, mailboxes are up).
        // Runs before the PDO-mapping dump below so the logged layout
        // reflects any remapping done here. A failed write aborts init:
        // these are things like 0x6060 = 8 (CSP) — silently running a
        // drive in the wrong mode is worse than not starting.
        for (pos, sd) in group.iter(&maindevice).enumerate() {
            let Some(cfg) = slaves.iter().find(|s| s.index == pos as u16) else {
                continue;
            };
            for cmd in &cfg.init_sdo {
                let res = match cmd.bits {
                    8 => {
                        sd.sdo_write(cmd.index, cmd.sub_index, cmd.value as u8)
                            .await
                    }
                    16 => {
                        sd.sdo_write(cmd.index, cmd.sub_index, cmd.value as u16)
                            .await
                    }
                    // validate_init_sdo limited bits to {8, 16, 32}.
                    _ => {
                        sd.sdo_write(cmd.index, cmd.sub_index, cmd.value as u32)
                            .await
                    }
                };
                match res {
                    Ok(()) => tracing::info!(
                        slave = pos,
                        obj = format!("{:#06x}:{:02x}", cmd.index, cmd.sub_index),
                        value = cmd.value,
                        bits = cmd.bits,
                        "init sdo write"
                    ),
                    Err(e) => {
                        let _ = init_tx.send(InitResult::Err(format!(
                            "init sdo write {:#06x}:{:02x} = {} ({} bits) on slave {pos}: {e:?}",
                            cmd.index, cmd.sub_index, cmd.value, cmd.bits
                        )));
                        return;
                    }
                }
            }
        }

        // One-time: read + log the CoE PDO mapping (0x1C12 RxPDO-assign /
        // 0x1C13 TxPDO-assign -> 0x16xx / 0x1Axx entries). Surfaces the exact
        // byte offset of controlword / target_velocity / statusword / etc. in
        // the logs, so iomap channels are configured off the real layout
        // rather than guessed. Reads happen in PRE-OP where CoE is available.
        for sd in group.iter(&maindevice) {
            for (assign, dir) in [(0x1C12u16, "out/rxpdo"), (0x1C13u16, "in/txpdo")] {
                let count: u8 = sd.sdo_read(assign, 0u8).await.unwrap_or(0);
                let mut bit_off: u32 = 0;
                for i in 1..=count {
                    let pdo: u16 = match sd.sdo_read(assign, i).await {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let entries: u8 = sd.sdo_read(pdo, 0u8).await.unwrap_or(0);
                    for j in 1..=entries {
                        let entry: u32 = match sd.sdo_read(pdo, j).await {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let obj = (entry >> 16) as u16;
                        let sub = ((entry >> 8) & 0xff) as u8;
                        let bits = (entry & 0xff) as u8;
                        tracing::info!(
                            dir,
                            pdo = format!("{pdo:#06x}"),
                            obj = format!("{obj:#06x}:{sub:02x}"),
                            bits,
                            byte = bit_off / 8,
                            "pdo entry"
                        );
                        bit_off += bits as u32;
                    }
                }
            }
        }

        let sync0 = Duration::from_micros(cycle_us as u64);

        // Effective DC mode per SubDevice: per-slave override if listed in
        // the config, else the device-level default. The bus takes the DC
        // path when any SubDevice ends up Sync0 — SubDevices left Off
        // aren't flagged, and ethercrab's configure_dc_sync skips unflagged
        // ones, so plain IO couplers free-run inside a DC bus.
        let effective_dc = |pos: u16| {
            slaves
                .iter()
                .find(|s| s.index == pos)
                .and_then(|s| s.dc_sync)
                .unwrap_or(dc_sync)
        };
        let subdevice_count = group.iter(&maindevice).count() as u16;
        let bus_dc = if (0..subdevice_count).any(|p| effective_dc(p) == EthercatDcSync::Sync0) {
            EthercatDcSync::Sync0
        } else {
            EthercatDcSync::Off
        };

        match bus_dc {
            EthercatDcSync::Sync0 => {
                // Servo drives (e.g. Inovance SV660N) need DC SYNC0 to
                // reach OP. Flag SYNC0 on the SubDevices that want it,
                // configure the group DC, then *request* OP and cycle
                // tx_rx_dc until all OP — a blocking into_op() doesn't pump
                // PDI, so the drive's SyncManager watchdog would trip
                // during SAFE-OP -> OP.
                for (pos, mut subdevice) in group.iter_mut(&maindevice).enumerate() {
                    if effective_dc(pos as u16) == EthercatDcSync::Sync0 {
                        subdevice.set_dc_sync(DcSync::Sync0);
                    }
                }
                let group = match group.into_pre_op_pdi(&maindevice).await {
                    Ok(g) => g,
                    Err(e) => {
                        let _ = init_tx.send(InitResult::Err(format!(
                            "into_pre_op_pdi (PRE-OP+PDI): {e:?}"
                        )));
                        return;
                    }
                };
                let group = match group
                    .configure_dc_sync(
                        &maindevice,
                        DcConfiguration {
                            // Start SYNC0 100ms out; period = the cycle; send
                            // data half-way through the cycle.
                            start_delay: Duration::from_millis(100),
                            sync0_period: sync0,
                            sync0_shift: sync0 / 2,
                        },
                    )
                    .await
                {
                    Ok(g) => g,
                    Err(e) => {
                        let _ = init_tx.send(InitResult::Err(format!("configure_dc_sync: {e:?}")));
                        return;
                    }
                };
                let group = match group.request_into_op(&maindevice).await {
                    Ok(g) => g,
                    Err(e) => {
                        let _ = init_tx.send(InitResult::Err(format!(
                            "request_into_op (-> request OP): {e:?}"
                        )));
                        return;
                    }
                };

                // Capture discovery before confirming OP so the topology is
                // visible even if OP never settles.
                let discovered = capture_discovery!(group, maindevice, pdi);

                // Pump tx_rx_dc until every SubDevice reaches OP (zero
                // outputs / controlword 0 — nothing moves). Bounded.
                {
                    let deadline = std::time::Instant::now() + Duration::from_secs(10);
                    let mut reached_op = false;
                    while std::time::Instant::now() < deadline {
                        match group.tx_rx_dc(&maindevice).await {
                            Ok(resp) => {
                                if resp.all_op() {
                                    reached_op = true;
                                    break;
                                }
                            }
                            Err(e) => tracing::warn!(?e, "tx_rx_dc while waiting for OP"),
                        }
                        smol::Timer::after(sync0).await;
                    }
                    if !reached_op {
                        let _ = init_tx.send(InitResult::Err(
                            "SubDevices did not reach OP within 10s (SyncManager watchdog / DC?)"
                                .into(),
                        ));
                        return;
                    }
                    tracing::info!("all subdevices reached OP (dc=sync0)");
                }

                let _ = init_tx.send(InitResult::Ok { discovered });

                // DC cyclic loop: tx_rx_dc keeps the reference clock synced
                // and its CycleInfo tells us when to send the next frame
                // (stays aligned to SYNC0).
                let mut health = HealthTracker::with_flag(UNHEALTHY_AFTER_TX_ERRORS, healthy);
                // Tracks whether the *previous* exchange succeeded — the gear
                // engines read inputs captured by that exchange, so on a
                // failed cycle they freeze (no master advance / target held)
                // and the bus recovers without a one-cycle catch-up step.
                let mut bus_ok = true;
                while !shutdown.load(Ordering::Relaxed) {
                    let cycle_start = std::time::Instant::now();
                    copy_outputs_to_bus!(group, maindevice, pdi);
                    gear_tick!(group, maindevice, engines, bus_ok);
                    let next_wait = match group.tx_rx_dc(&maindevice).await {
                        Ok(resp) => {
                            copy_inputs_from_bus!(group, maindevice, pdi);
                            if health.record_success() == HealthTransition::Recovered {
                                tracing::info!("ethercat recovered; cyclic exchange running again");
                            }
                            bus_ok = true;
                            resp.extra.next_cycle_wait
                        }
                        Err(e) => {
                            note_txrx_failure(&mut health, &e);
                            bus_ok = false;
                            sync0
                        }
                    };
                    smol::Timer::at(cycle_start + next_wait).await;
                }
                // Final flush before teardown: failsafe has zeroed the
                // output mirror; push it out once more so the drive latches
                // controlword = 0 (Disable Voltage) before the thread stops,
                // instead of de-energizing only via its own SyncManager
                // watchdog once the master goes away.
                copy_outputs_to_bus!(group, maindevice, pdi);
                let _ = group.tx_rx_dc(&maindevice).await;
                tracing::info!("ethercat cyclic loop exiting (shutdown signalled)");
            }

            EthercatDcSync::Off => {
                // Free-run (no DC): a blocking into_op works for IO couplers
                // / SubDevices that don't need (or can't do) DC. Then a
                // fixed-interval tx_rx loop.
                let group = match group.into_op(&maindevice).await {
                    Ok(g) => g,
                    Err(e) => {
                        let _ =
                            init_tx.send(InitResult::Err(format!("into_op (PRE-OP -> OP): {e:?}")));
                        return;
                    }
                };

                let discovered = capture_discovery!(group, maindevice, pdi);
                let _ = init_tx.send(InitResult::Ok { discovered });

                let mut tick = smol::Timer::interval(sync0);
                use smol::stream::StreamExt;
                let mut health = HealthTracker::with_flag(UNHEALTHY_AFTER_TX_ERRORS, healthy);
                let mut bus_ok = true;
                while !shutdown.load(Ordering::Relaxed) {
                    copy_outputs_to_bus!(group, maindevice, pdi);
                    gear_tick!(group, maindevice, engines, bus_ok);
                    match group.tx_rx(&maindevice).await {
                        Ok(_) => {
                            if health.record_success() == HealthTransition::Recovered {
                                tracing::info!("ethercat recovered; cyclic exchange running again");
                            }
                            bus_ok = true;
                        }
                        Err(e) => {
                            note_txrx_failure(&mut health, &e);
                            bus_ok = false;
                        }
                    }
                    copy_inputs_from_bus!(group, maindevice, pdi);
                    tick.next().await;
                }
                // Final flush before teardown: failsafe has zeroed the
                // output mirror; push it out once more so the SubDevices
                // latch their safe (zero) outputs before the thread stops.
                copy_outputs_to_bus!(group, maindevice, pdi);
                let _ = group.tx_rx(&maindevice).await;
                tracing::info!("ethercat cyclic loop exiting (shutdown signalled)");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // The bus-side paths need a real NIC + CAP_NET_RAW, so these exercise
    // the bounded-join logic in isolation — that's the part that has to
    // hold the line on shutdown latency regardless of bus health.

    #[test]
    fn join_worker_joins_a_thread_that_stops() {
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let worker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            flag.store(true, Ordering::Relaxed);
        });

        let start = Instant::now();
        assert!(
            join_worker(&stopped, Some(worker), WORKER_JOIN_TIMEOUT),
            "should report a successful join once the worker stops"
        );
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "join should return promptly after the worker stops"
        );
    }

    #[test]
    fn join_worker_returns_immediately_if_already_stopped() {
        let stopped = Arc::new(AtomicBool::new(true));
        let start = Instant::now();
        // No handle: the common Drop-already-ran case must not block.
        assert!(join_worker(&stopped, None, WORKER_JOIN_TIMEOUT));
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn join_worker_abandons_a_thread_that_never_stops() {
        let stopped = Arc::new(AtomicBool::new(false));
        // A worker that outlives the (short) timeout and never flags stop.
        let worker = thread::spawn(|| thread::sleep(Duration::from_millis(500)));

        let start = Instant::now();
        assert!(
            !join_worker(&stopped, Some(worker), Duration::from_millis(40)),
            "should give up rather than block on a worker that won't stop"
        );
        assert!(
            start.elapsed() < Duration::from_millis(300),
            "must return shortly after the timeout, not wait out the worker"
        );
    }
}
