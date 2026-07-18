//! Bus transport abstraction + the `_sim` in-memory implementation.
//!
//! `CanBus` is the seam between the adapter's protocol logic and the
//! wire: SocketCAN on a Linux edge (see `socketcan_bus`), or `SimBus`
//! everywhere for dev machines and tests. The sim doesn't just loop
//! frames back — it runs a small CANopen *slave* task (object
//! dictionary + SDO server + heartbeat producer + PDO engine) so the
//! full master-side protocol path is exercised end to end.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use iocore::IoError;
use project::{CanopenConfig, CanopenTransport};
use tokio::sync::mpsc;

use crate::frame::{self, CanFrame, NmtState};

#[async_trait]
pub trait CanBus: Send {
    async fn send(&mut self, frame: CanFrame) -> Result<(), IoError>;
    /// Next frame off the bus. Long-poll — the caller select!s on it.
    async fn recv(&mut self) -> Result<CanFrame, IoError>;
}

// ---------------- Sim ----------------

/// Master-side handle to the simulated bus.
pub struct SimBus {
    to_slave: mpsc::Sender<CanFrame>,
    from_slave: mpsc::Receiver<CanFrame>,
    slave_task: tokio::task::JoinHandle<()>,
}

impl Drop for SimBus {
    fn drop(&mut self) {
        self.slave_task.abort();
    }
}

#[async_trait]
impl CanBus for SimBus {
    async fn send(&mut self, frame: CanFrame) -> Result<(), IoError> {
        self.to_slave
            .send(frame)
            .await
            .map_err(|_| IoError::Transport("sim bus closed".into()))
    }

    async fn recv(&mut self) -> Result<CanFrame, IoError> {
        self.from_slave
            .recv()
            .await
            .ok_or_else(|| IoError::Transport("sim bus closed".into()))
    }
}

impl SimBus {
    /// Bring up the bus with one simulated slave shaped after `config`:
    /// its object dictionary holds every configured channel's
    /// `index:sub` (zero-initialised), its heartbeat runs at 200 ms,
    /// and PDO-transport channels ride the configured slots. Writes
    /// (SDO or RPDO) land in the dictionary, and TPDOs are packed from
    /// it — so what the master writes is what it reads back, which is
    /// exactly the loop tests and demo projects need.
    pub fn connect(config: &CanopenConfig) -> SimBus {
        let (to_slave, slave_rx) = mpsc::channel::<CanFrame>(64);
        let (slave_tx, from_slave) = mpsc::channel::<CanFrame>(64);
        let slave = SimSlave::from_config(config);
        let slave_task = tokio::spawn(slave.run(slave_rx, slave_tx));
        SimBus {
            to_slave,
            from_slave,
            slave_task,
        }
    }
}

/// One simulated CANopen node.
struct SimSlave {
    node: u8,
    state: NmtState,
    /// Object dictionary: (index, sub) → little-endian value bytes.
    od: HashMap<(u16, u8), Vec<u8>>,
    /// TPDO layouts from the config: slot → [(byte_offset, len, index, sub)].
    tpdos: HashMap<u8, Vec<(u8, usize, u16, u8)>>,
    /// RPDO layouts, same shape.
    rpdos: HashMap<u8, Vec<(u8, usize, u16, u8)>>,
}

impl SimSlave {
    fn from_config(config: &CanopenConfig) -> Self {
        let mut od = HashMap::new();
        let mut tpdos: HashMap<u8, Vec<(u8, usize, u16, u8)>> = HashMap::new();
        let mut rpdos: HashMap<u8, Vec<(u8, usize, u16, u8)>> = HashMap::new();
        for ch in &config.channels {
            let len = frame::type_len(ch.data_type);
            od.insert((ch.index, ch.sub_index), vec![0u8; len]);
            match ch.transport {
                CanopenTransport::Tpdo { slot, byte_offset } => {
                    tpdos
                        .entry(slot)
                        .or_default()
                        .push((byte_offset, len, ch.index, ch.sub_index));
                }
                CanopenTransport::Rpdo { slot, byte_offset } => {
                    rpdos
                        .entry(slot)
                        .or_default()
                        .push((byte_offset, len, ch.index, ch.sub_index));
                }
                CanopenTransport::Sdo => {}
            }
        }
        SimSlave {
            node: config.node_id,
            state: NmtState::PreOperational,
            od,
            tpdos,
            rpdos,
        }
    }

    async fn run(mut self, mut rx: mpsc::Receiver<CanFrame>, tx: mpsc::Sender<CanFrame>) {
        // Heartbeat + TPDO cadence. 50 ms keeps tests fast while still
        // exercising the periodic paths; a real node's rates come from
        // its 0x1017 / event timers.
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                maybe = rx.recv() => {
                    let Some(f) = maybe else { return };
                    if let Some(reply) = self.handle(&f) {
                        if tx.send(reply).await.is_err() { return; }
                    }
                }
                _ = tick.tick() => {
                    // Test hook: SDO-writing 1 to 0x5FFF:00 mutes the
                    // heartbeat producer, simulating a dead node so the
                    // master's watchdog path can be exercised without
                    // pulling a real cable. Harmless in demos (the
                    // object doesn't exist unless a channel maps it).
                    let muted = self
                        .od
                        .get(&(0x5FFF, 0))
                        .is_some_and(|b| b.first().copied().unwrap_or(0) != 0);
                    if !muted {
                        let hb = CanFrame::new(
                            frame::cob::heartbeat(self.node),
                            &[state_byte(self.state)],
                        );
                        if tx.send(hb).await.is_err() { return; }
                    }
                    // TPDOs only run in Operational — exactly the
                    // detail that makes NMT start worth testing. A muted
                    // (test-dead) node sends nothing at all.
                    if !muted && self.state == NmtState::Operational {
                        for (slot, entries) in &self.tpdos {
                            let mut data = [0u8; 8];
                            let mut used = 0usize;
                            for (off, len, idx, sub) in entries {
                                if let Some(bytes) = self.od.get(&(*idx, *sub)) {
                                    let end = (*off as usize + len).min(8);
                                    data[*off as usize..end]
                                        .copy_from_slice(&bytes[..end - *off as usize]);
                                    used = used.max(end);
                                }
                            }
                            let f = CanFrame::new(
                                frame::cob::tpdo(*slot, self.node),
                                &data[..used.max(1)],
                            );
                            if tx.send(f).await.is_err() { return; }
                        }
                    }
                }
            }
        }
    }

    fn handle(&mut self, f: &CanFrame) -> Option<CanFrame> {
        // NMT (broadcast or addressed to us).
        if f.id == frame::cob::NMT && f.len >= 2 && (f.data[1] == 0 || f.data[1] == self.node) {
            self.state = match f.data[0] {
                0x01 => NmtState::Operational,
                0x02 => NmtState::Stopped,
                0x80 => NmtState::PreOperational,
                0x81 | 0x82 => NmtState::PreOperational, // reset lands back in pre-op
                _ => self.state,
            };
            return None;
        }
        // RPDO writes → dictionary.
        for slot in 1..=4u8 {
            if f.id == frame::cob::rpdo(slot, self.node) {
                if self.state != NmtState::Operational {
                    return None; // PDOs are inert outside Operational
                }
                if let Some(entries) = self.rpdos.get(&slot) {
                    for (off, len, idx, sub) in entries.clone() {
                        let start = off as usize;
                        if start + len <= f.len as usize {
                            self.od
                                .insert((idx, sub), f.data[start..start + len].to_vec());
                        }
                    }
                }
                return None;
            }
        }
        // SDO server.
        if f.id == frame::cob::sdo_request(self.node) && f.len >= 8 {
            return Some(self.handle_sdo(f));
        }
        None
    }

    fn handle_sdo(&mut self, f: &CanFrame) -> CanFrame {
        let cmd = f.data[0];
        let index = u16::from_le_bytes([f.data[1], f.data[2]]);
        let sub = f.data[3];
        let resp_id = frame::cob::sdo_response(self.node);
        let abort = |code: u32| {
            let [b0, b1, b2, b3] = code.to_le_bytes();
            CanFrame::new(resp_id, &[0x80, f.data[1], f.data[2], sub, b0, b1, b2, b3])
        };
        match cmd >> 5 {
            // upload (read)
            2 => match self.od.get(&(index, sub)) {
                Some(bytes) => {
                    let n = (4 - bytes.len()) as u8;
                    let scs = 0x43 | (n << 2); // expedited + size indicated
                    let mut payload = [scs, f.data[1], f.data[2], sub, 0, 0, 0, 0];
                    payload[4..4 + bytes.len()].copy_from_slice(bytes);
                    CanFrame::new(resp_id, &payload)
                }
                None => abort(0x0602_0000),
            },
            // download (write)
            1 => {
                if !self.od.contains_key(&(index, sub)) {
                    return abort(0x0602_0000);
                }
                let n = ((cmd >> 2) & 0x03) as usize;
                let len = 4 - n;
                self.od.insert((index, sub), f.data[4..4 + len].to_vec());
                CanFrame::new(resp_id, &[0x60, f.data[1], f.data[2], sub, 0, 0, 0, 0])
            }
            _ => abort(0x0504_0001),
        }
    }
}

fn state_byte(s: NmtState) -> u8 {
    match s {
        NmtState::BootUp => 0x00,
        NmtState::Stopped => 0x04,
        NmtState::Operational => 0x05,
        NmtState::PreOperational => 0x7F,
        NmtState::Unknown(b) => b,
    }
}
