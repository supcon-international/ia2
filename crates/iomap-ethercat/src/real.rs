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
use std::time::Duration;

use async_trait::async_trait;
use ethercrab::{std::ethercat_now, MainDevice, MainDeviceConfig, PduStorage, Timeouts};
use iocore::{ChannelValue, IoDevice, IoError};
use project::{EthercatChannel, EthercatConfig, EthercatPdoDirection};

use crate::bits;

// Storage sizing — picked to comfortably cover a typical edge configuration
// (an EK1100-class coupler + ~30 EL modules). MAX_PDU_DATA at ~1100 matches
// the upstream examples; PDI_LEN at 256 covers most fieldbus surfaces.
// MAX_SUBDEVICES must be a power of 2 > 1.
const MAX_SUBDEVICES: usize = 32;
const MAX_PDU_DATA: usize = PduStorage::element_size(1100);
const MAX_FRAMES: usize = 16;
const PDI_LEN: usize = 256;

type Storage = PduStorage<MAX_FRAMES, MAX_PDU_DATA>;

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

pub struct RealEthercat {
    name: String,
    channels: HashMap<String, EthercatChannel>,
    pdi: Arc<Mutex<PdiMirror>>,
    shutdown: Arc<AtomicBool>,
    // Kept so we don't drop the JoinHandle silently. Not joined on drop —
    // the cyclic loop exits cooperatively when `shutdown` flips.
    _thread: Option<thread::JoinHandle<()>>,
}

/// Whether the bus-side init succeeded. Sent back over the oneshot.
#[derive(Debug)]
enum InitResult {
    Ok { discovered: Vec<SlaveDiscovery> },
    Err(String),
}

#[derive(Debug, Clone)]
pub struct SlaveDiscovery {
    pub index: u16,
    pub name: String,
    pub input_bytes: u16,
    pub output_bytes: u16,
    pub vendor_id: u32,
    pub product_id: u32,
}

impl RealEthercat {
    pub async fn connect(name: String, config: &EthercatConfig) -> Result<Self, IoError> {
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

        let channels: HashMap<String, EthercatChannel> = config
            .channels
            .iter()
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        let pdi = Arc::new(Mutex::new(PdiMirror::default()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (init_tx, init_rx) = mpsc::sync_channel::<InitResult>(1);

        let nic = config.nic.clone();
        let cycle_us = config.cycle_us.max(100); // hard floor: don't melt the CPU
        let pdi_clone = pdi.clone();
        let shutdown_clone = shutdown.clone();
        let thread_name = format!("ec-{name}");

        let thread = thread::Builder::new()
            .name(thread_name)
            .spawn(move || smol_main(&nic, cycle_us, pdi_clone, shutdown_clone, init_tx))
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
                tracing::info!(
                    name = %name,
                    nic = %config.nic,
                    cycle_us,
                    discovered = discovered.len(),
                    "ethercat device live"
                );
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
                Ok(Self {
                    name,
                    channels,
                    pdi,
                    shutdown,
                    _thread: Some(thread),
                })
            }
            InitResult::Err(e) => Err(IoError::Connect(format!("ethercat init: {e}"))),
        }
    }
}

impl Drop for RealEthercat {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // Don't join — drop can happen during shutdown sequences and we
        // never want to block the runtime here. The worker exits on the
        // next cycle tick.
    }
}

#[async_trait]
impl IoDevice for RealEthercat {
    fn name(&self) -> &str {
        &self.name
    }

    async fn read_channel(&mut self, channel: &str) -> Result<ChannelValue, IoError> {
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
        let mut pdi = self.pdi.lock().expect("pdi mirror poisoned");
        for buf in pdi.outputs.values_mut() {
            buf.fill(0);
        }
        tracing::info!(device = %self.name, "ethercat output PDI zeroed for failsafe");
        Ok(())
    }
}

/// The entire ethercrab session lives inside this function — bus walk,
/// state-machine transition to OP, and the cyclic exchange loop. Runs on
/// its own thread under `smol::block_on` to drive `async-io`.
fn smol_main(
    nic: &str,
    cycle_us: u32,
    pdi: Arc<Mutex<PdiMirror>>,
    shutdown: Arc<AtomicBool>,
    init_tx: mpsc::SyncSender<InitResult>,
) {
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
        MainDeviceConfig::default(),
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
        let group = match maindevice
            .init_single_group::<MAX_SUBDEVICES, PDI_LEN>(ethercat_now)
            .await
        {
            Ok(g) => g,
            Err(e) => {
                let _ = init_tx.send(InitResult::Err(format!("init_single_group: {e:?}")));
                return;
            }
        };

        // PRE-OP → OP transition. ethercrab handles SafeOp internally.
        let group = match group.into_op(&maindevice).await {
            Ok(g) => g,
            Err(e) => {
                let _ = init_tx.send(InitResult::Err(format!("into_op (PRE-OP -> OP): {e:?}")));
                return;
            }
        };

        // Discovery report + size the PDI mirror.
        let mut discovered: Vec<SlaveDiscovery> = Vec::new();
        {
            let mut mirror = pdi.lock().expect("pdi mirror poisoned");
            for (offset_idx, sd) in group.iter(&maindevice).enumerate() {
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
        let _ = init_tx.send(InitResult::Ok {
            discovered: discovered.clone(),
        });

        // Cyclic loop. We use smol::Timer rather than tokio so we stay
        // on this thread's reactor. Cycle time is a soft target — under
        // overload we let the bus pace us via tx_rx().
        let mut tick = smol::Timer::interval(Duration::from_micros(cycle_us as u64));
        use smol::stream::StreamExt;
        while !shutdown.load(Ordering::Relaxed) {
            // Pre-cycle: copy our owned output bytes onto the bus surface.
            {
                let mirror = pdi.lock().expect("pdi mirror poisoned");
                for (offset_idx, sd) in group.iter(&maindevice).enumerate() {
                    let idx = offset_idx as u16;
                    if let Some(src) = mirror.outputs.get(&idx) {
                        let mut out = sd.outputs_raw_mut();
                        let n = out.len().min(src.len());
                        out[..n].copy_from_slice(&src[..n]);
                    }
                }
            }

            // The actual bus exchange.
            if let Err(e) = group.tx_rx(&maindevice).await {
                // Don't kill the device on a transient error — log and
                // re-tick. Persistent failures will show up as stale
                // mirror values and warning spam, which is the right
                // signal for the user.
                tracing::warn!(?e, "ethercat tx_rx failed");
            }

            // Post-cycle: snapshot inputs back into our mirror.
            {
                let mut mirror = pdi.lock().expect("pdi mirror poisoned");
                for (offset_idx, sd) in group.iter(&maindevice).enumerate() {
                    let idx = offset_idx as u16;
                    let inputs = sd.inputs_raw();
                    if let Some(dst) = mirror.inputs.get_mut(&idx) {
                        let n = inputs.len().min(dst.len());
                        dst[..n].copy_from_slice(&inputs[..n]);
                    }
                }
            }

            tick.next().await;
        }
        tracing::info!("ethercat cyclic loop exiting (shutdown signalled)");
    });
}
