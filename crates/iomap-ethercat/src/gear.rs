//! In-cycle electronic gear (B-tier motion).
//!
//! The ST-tier gear (see the eg_servo / dual_gear example projects) computes
//! the follower's target_position in the PLC scan plane (~500 Hz, async to
//! the bus), so up to a scan of phase jitter sits between "target computed"
//! and "frame sent". This module moves the per-cycle interpolation into the
//! cyclic loop itself: the slow plane only feeds parameters, and the target
//! is generated strictly cycle-aligned.
//!
//! Split:
//!   - [`GearShared`] — lock-free parameter mailbox. `write_channel` routes
//!     the configured channel names here (instead of PDI bytes); the cyclic
//!     loop reads it every tick. Engine state (engaged / trip) flows back
//!     the same way for `read_channel`.
//!   - [`GearEngine`] — pure per-cycle math, no I/O: callers hand it the
//!     follower's statusword + actual_position and (for an axis master) the
//!     master's actual_position, and get the target_position to write.
//!     Pure so the real Sync0 loop, the free-run loop, the sim ticker and
//!     unit tests all share one implementation.
//!
//! Safety model (carried over from the field-hardened ST tier, enforced in
//! the fast plane so no slow-plane mistake can bypass it):
//!   - While the follower is not in CiA402 Operation Enabled the engine
//!     shadows its actual_position — zero target jump at enable.
//!   - Engagement requires `max_travel` > 0 (locked by default) and latches
//!     the master/follower origins at the engage edge; overtravel trips to
//!     a position hold, cleared only by dropping the engage channel.
//!   - Losing Operation Enabled mid-run demands a re-arm: engage must be
//!     observed low before a new engagement, so a stale engage force can't
//!     auto-restart motion after a fault reset.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use iocore::{ChannelValue, IoError};
use project::{EthercatGear, GearMaster};

/// Lock-free slow→fast parameter mailbox plus fast→slow feedback, one per
/// configured gear axis. f64 values are stored as `to_bits` in `AtomicU64`.
#[derive(Debug, Default)]
pub struct GearShared {
    ratio_num: AtomicU64,
    ratio_den: AtomicU64,
    ratio_step: AtomicU64,
    phase_ofs: AtomicU64,
    master_vel: AtomicU64,
    max_travel: AtomicU64,
    engage: AtomicBool,
    // Engine → slow plane.
    engaged_fb: AtomicBool,
    trip_fb: AtomicBool,
}

/// Writable gear parameters (slow plane → engine).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GearParam {
    Engage,
    RatioNum,
    RatioDen,
    RatioStep,
    PhaseOfs,
    MasterVel,
    MaxTravel,
}

/// Read-only engine state (engine → slow plane).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GearReadback {
    Engage,
    RatioNum,
    RatioDen,
    RatioStep,
    PhaseOfs,
    MasterVel,
    MaxTravel,
    Engaged,
    Trip,
}

impl GearShared {
    fn store_f64(a: &AtomicU64, v: f64) {
        a.store(v.to_bits(), Ordering::Relaxed);
    }
    fn load_f64(a: &AtomicU64) -> f64 {
        f64::from_bits(a.load(Ordering::Relaxed))
    }

    /// Slow-plane write. Numeric `ChannelValue`s are widened to f64; the
    /// engage flag accepts Bool or any numeric (nonzero = engaged) so REAL
    /// or INT PLC vars can drive it too.
    pub fn write(&self, param: GearParam, value: &ChannelValue) {
        let as_f64 = match *value {
            ChannelValue::Bool(b) => {
                if b {
                    1.0
                } else {
                    0.0
                }
            }
            ChannelValue::U16(v) => v as f64,
            ChannelValue::I32(v) => v as f64,
            ChannelValue::Real(v) => v as f64,
            ChannelValue::F64(v) => v,
        };
        match param {
            GearParam::Engage => self.engage.store(as_f64 != 0.0, Ordering::Relaxed),
            GearParam::RatioNum => Self::store_f64(&self.ratio_num, as_f64),
            GearParam::RatioDen => Self::store_f64(&self.ratio_den, as_f64),
            GearParam::RatioStep => Self::store_f64(&self.ratio_step, as_f64),
            GearParam::PhaseOfs => Self::store_f64(&self.phase_ofs, as_f64),
            GearParam::MasterVel => Self::store_f64(&self.master_vel, as_f64),
            GearParam::MaxTravel => Self::store_f64(&self.max_travel, as_f64),
        }
    }

    /// Slow-plane read: parameter echo plus live engine state.
    pub fn read(&self, rb: GearReadback) -> ChannelValue {
        match rb {
            GearReadback::Engage => ChannelValue::Bool(self.engage.load(Ordering::Relaxed)),
            GearReadback::RatioNum => ChannelValue::F64(Self::load_f64(&self.ratio_num)),
            GearReadback::RatioDen => ChannelValue::F64(Self::load_f64(&self.ratio_den)),
            GearReadback::RatioStep => ChannelValue::F64(Self::load_f64(&self.ratio_step)),
            GearReadback::PhaseOfs => ChannelValue::F64(Self::load_f64(&self.phase_ofs)),
            GearReadback::MasterVel => ChannelValue::F64(Self::load_f64(&self.master_vel)),
            GearReadback::MaxTravel => ChannelValue::F64(Self::load_f64(&self.max_travel)),
            GearReadback::Engaged => ChannelValue::Bool(self.engaged_fb.load(Ordering::Relaxed)),
            GearReadback::Trip => ChannelValue::Bool(self.trip_fb.load(Ordering::Relaxed)),
        }
    }

    /// Failsafe hook: drop the engage request so the engine disengages on
    /// its next tick (position hold / shadow) even if the slow plane never
    /// gets another word in.
    pub fn disengage(&self) {
        self.engage.store(false, Ordering::Relaxed);
    }
}

/// Master position source, resolved from [`project::GearMaster`].
#[derive(Debug, Clone, Copy)]
pub enum MasterSrc {
    /// Software accumulator advanced by `master_vel` counts per cycle.
    Virtual,
    /// Another axis's actual_position, read from its input PDI each cycle.
    Axis { slave_index: u16, offset: usize },
}

/// Per-cycle gear engine for one follower axis. Owned by the cyclic worker
/// (or sim ticker); everything it needs from the slow plane arrives through
/// the shared mailbox, so `tick` is lock-free and allocation-free.
pub struct GearEngine {
    pub follower_index: u16,
    pub target_off: usize,
    pub actual_off: usize,
    pub status_off: usize,
    pub master: MasterSrc,
    shared: Arc<GearShared>,
    /// Virtual master accumulator (counts). Advances every tick so an
    /// engaged follower sees a continuous master, exactly like a real axis.
    virt_master: f64,
    /// Current output target (counts). Shadow / gear law / hold all land
    /// here; it is the single source of what goes on the wire.
    target: f64,
    was_engaged: bool,
    m0: f64,
    s0: f64,
    ratio: f64,
    trip: bool,
    /// Set when Operation Enabled is lost; a fresh engagement requires the
    /// engage request to be observed low first (operator re-arm), so stale
    /// engage forces can't restart motion after a fault reset.
    need_rearm: bool,
}

impl GearEngine {
    pub fn new(cfg: &EthercatGear, shared: Arc<GearShared>) -> Self {
        let master = match cfg.master {
            GearMaster::Virtual => MasterSrc::Virtual,
            GearMaster::Axis {
                slave_index,
                actual_pos_offset,
            } => MasterSrc::Axis {
                slave_index,
                offset: actual_pos_offset as usize,
            },
        };
        GearEngine {
            follower_index: cfg.slave_index,
            target_off: cfg.target_pos_offset as usize,
            actual_off: cfg.actual_pos_offset as usize,
            status_off: cfg.status_word_offset as usize,
            master,
            shared,
            virt_master: 0.0,
            target: 0.0,
            was_engaged: false,
            m0: 0.0,
            s0: 0.0,
            ratio: 0.0,
            trip: false,
            need_rearm: false,
        }
    }

    /// One bus cycle. `master_actual` is `Some` for an axis master (read by
    /// the caller from that axis's input PDI) and `None` for virtual.
    /// Returns the target_position to write into the follower's output PDI.
    pub fn tick(
        &mut self,
        follower_sw: u16,
        follower_actual: i32,
        master_actual: Option<i32>,
    ) -> i32 {
        let engage_in = self.shared.engage.load(Ordering::Relaxed);
        let ratio_num = GearShared::load_f64(&self.shared.ratio_num);
        let ratio_den = GearShared::load_f64(&self.shared.ratio_den);
        let ratio_step = GearShared::load_f64(&self.shared.ratio_step).max(0.0);
        let phase_ofs = GearShared::load_f64(&self.shared.phase_ofs);
        let master_vel = GearShared::load_f64(&self.shared.master_vel);
        let max_travel = GearShared::load_f64(&self.shared.max_travel);

        // The virtual master advances every tick, engaged or not, exactly
        // like a real axis keeps moving whether or not anyone follows it.
        self.virt_master += master_vel;
        let master = match master_actual {
            Some(a) => a as f64,
            None => self.virt_master,
        };

        // CiA402: Operation Enabled = statusword & 0x6F == 0x27.
        let enabled = (follower_sw & 0x006F) == 0x0027;
        if !enabled {
            // Shadow the feedback so the drive latches target == actual at
            // the enable instant — no jump, no following-error trip.
            self.target = follower_actual as f64;
            self.was_engaged = false;
            self.ratio = 0.0;
            self.need_rearm = true;
        } else {
            if !engage_in {
                self.need_rearm = false; // observed low while enabled: re-armed
                self.trip = false; // dropping engage is the trip reset
            }
            let engage = engage_in && !self.trip && !self.need_rearm && max_travel > 0.0;
            if engage && !self.was_engaged {
                self.m0 = master;
                self.s0 = self.target;
                self.ratio = 0.0;
            }
            self.was_engaged = engage;
            if engage {
                let tgt_ratio = if ratio_den == 0.0 {
                    0.0
                } else {
                    ratio_num / ratio_den
                };
                let d = (tgt_ratio - self.ratio).clamp(-ratio_step, ratio_step);
                self.ratio += d;
                self.target = self.s0 + self.ratio * (master - self.m0) + phase_ofs;
                // Overtravel: latch the trip. This cycle's (just-past-limit)
                // target still goes out — overshoot is bounded to one cycle
                // of master motion × ratio; from the next tick we hold.
                if (self.target - self.s0).abs() >= max_travel {
                    self.trip = true;
                }
            }
            // Disengaged / tripped / re-arm pending: hold self.target.
        }

        self.shared
            .engaged_fb
            .store(self.was_engaged, Ordering::Relaxed);
        self.shared.trip_fb.store(self.trip, Ordering::Relaxed);

        // Saturate rather than wrap at the i32 edge (±256 motor revs on a
        // 23-bit encoder). Long unidirectional runs remain a known limit.
        self.target.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32
    }
}

/// Channel-name routing tables for one device's gear axes, consulted by
/// `write_channel` / `read_channel` before the PDI channel lookup.
#[derive(Default)]
pub struct GearRouting {
    pub writes: HashMap<String, (Arc<GearShared>, GearParam)>,
    pub reads: HashMap<String, (Arc<GearShared>, GearReadback)>,
}

impl GearRouting {
    pub fn is_empty(&self) -> bool {
        self.writes.is_empty() && self.reads.is_empty()
    }

    /// Failsafe hook: drop every gear's engage request. The engines pick
    /// this up on their next tick and fall back to position hold / shadow.
    pub fn disengage_all(&self) {
        for (shared, _) in self.writes.values() {
            shared.disengage();
        }
    }

    pub fn write(&self, channel: &str, value: &ChannelValue) -> Option<Result<(), IoError>> {
        self.writes.get(channel).map(|(shared, param)| {
            shared.write(*param, value);
            Ok(())
        })
    }

    pub fn read(&self, channel: &str) -> Option<ChannelValue> {
        self.reads.get(channel).map(|(shared, rb)| shared.read(*rb))
    }
}

/// Build the engines + routing tables from the device config. Returns the
/// engines for the cyclic worker and the routing for the device facade.
pub fn build(gears: &[EthercatGear]) -> (Vec<GearEngine>, GearRouting) {
    let mut engines = Vec::with_capacity(gears.len());
    let mut routing = GearRouting::default();
    for cfg in gears {
        let shared = Arc::new(GearShared::default());
        let w = &mut routing.writes;
        w.insert(
            cfg.engage_channel.clone(),
            (shared.clone(), GearParam::Engage),
        );
        w.insert(
            cfg.ratio_num_channel.clone(),
            (shared.clone(), GearParam::RatioNum),
        );
        w.insert(
            cfg.ratio_den_channel.clone(),
            (shared.clone(), GearParam::RatioDen),
        );
        w.insert(
            cfg.ratio_step_channel.clone(),
            (shared.clone(), GearParam::RatioStep),
        );
        w.insert(
            cfg.phase_channel.clone(),
            (shared.clone(), GearParam::PhaseOfs),
        );
        w.insert(
            cfg.master_vel_channel.clone(),
            (shared.clone(), GearParam::MasterVel),
        );
        w.insert(
            cfg.max_travel_channel.clone(),
            (shared.clone(), GearParam::MaxTravel),
        );
        let r = &mut routing.reads;
        r.insert(
            cfg.engaged_channel.clone(),
            (shared.clone(), GearReadback::Engaged),
        );
        r.insert(
            cfg.trip_channel.clone(),
            (shared.clone(), GearReadback::Trip),
        );
        // Parameter echoes so an iomap input can observe what's in force.
        r.insert(
            cfg.engage_channel.clone(),
            (shared.clone(), GearReadback::Engage),
        );
        r.insert(
            cfg.ratio_num_channel.clone(),
            (shared.clone(), GearReadback::RatioNum),
        );
        r.insert(
            cfg.ratio_den_channel.clone(),
            (shared.clone(), GearReadback::RatioDen),
        );
        r.insert(
            cfg.ratio_step_channel.clone(),
            (shared.clone(), GearReadback::RatioStep),
        );
        r.insert(
            cfg.phase_channel.clone(),
            (shared.clone(), GearReadback::PhaseOfs),
        );
        r.insert(
            cfg.master_vel_channel.clone(),
            (shared.clone(), GearReadback::MasterVel),
        );
        r.insert(
            cfg.max_travel_channel.clone(),
            (shared.clone(), GearReadback::MaxTravel),
        );
        engines.push(GearEngine::new(cfg, shared));
    }
    (engines, routing)
}

/// Little-endian i32 read from an input PDI buffer; `None` if out of range.
pub fn read_i32(buf: &[u8], off: usize) -> Option<i32> {
    buf.get(off..off + 4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Little-endian u16 read from an input PDI buffer; `None` if out of range.
pub fn read_u16(buf: &[u8], off: usize) -> Option<u16> {
    buf.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

/// Little-endian i32 write into an output PDI buffer; ignored if out of range
/// (misconfigured offset — the validation pass warns at connect).
pub fn write_i32(buf: &mut [u8], off: usize, v: i32) {
    if let Some(dst) = buf.get_mut(off..off + 4) {
        dst.copy_from_slice(&v.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OP_ENABLED: u16 = 0x1637; // real SV660N Operation Enabled word
    const RDY_ONLY: u16 = 0x1631; // Ready To Switch On (not enabled)

    fn engine_virtual() -> (GearEngine, Arc<GearShared>) {
        let cfg = EthercatGear {
            slave_index: 0,
            target_pos_offset: 2,
            actual_pos_offset: 4,
            status_word_offset: 2,
            master: GearMaster::Virtual,
            engage_channel: "gear_engage".into(),
            ratio_num_channel: "ratio_num".into(),
            ratio_den_channel: "ratio_den".into(),
            ratio_step_channel: "ratio_step".into(),
            phase_channel: "phase_ofs".into(),
            master_vel_channel: "master_vel".into(),
            max_travel_channel: "gear_max_travel".into(),
            engaged_channel: "gear_engaged".into(),
            trip_channel: "gear_trip".into(),
        };
        let shared = Arc::new(GearShared::default());
        (GearEngine::new(&cfg, shared.clone()), shared)
    }

    fn set(shared: &GearShared, p: GearParam, v: f64) {
        shared.write(p, &ChannelValue::F64(v));
    }

    #[test]
    fn shadows_actual_until_enabled_then_holds() {
        let (mut e, _s) = engine_virtual();
        // Not enabled: target shadows whatever the feedback says.
        assert_eq!(e.tick(RDY_ONLY, 12345, None), 12345);
        assert_eq!(e.tick(RDY_ONLY, -777, None), -777);
        // Enable: target latched (holds), no jump.
        assert_eq!(e.tick(OP_ENABLED, -777, None), -777);
        assert_eq!(e.tick(OP_ENABLED, -700, None), -777, "hold, not follow");
    }

    #[test]
    fn engage_refused_without_max_travel() {
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 1.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 1.0);
        set(&s, GearParam::MasterVel, 100.0);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        for _ in 0..10 {
            assert_eq!(e.tick(OP_ENABLED, 0, None), 0, "locked while max_travel=0");
        }
        assert_eq!(s.read(GearReadback::Engaged), ChannelValue::Bool(false));
    }

    #[test]
    fn steady_state_ratio_exact() {
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 2.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 1.0); // hard engage: full ratio in 1 tick
        set(&s, GearParam::MasterVel, 100.0);
        set(&s, GearParam::MaxTravel, 1e9);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        let t1 = e.tick(OP_ENABLED, 0, None);
        let mut prev = t1;
        for _ in 0..50 {
            let t = e.tick(OP_ENABLED, 0, None);
            assert_eq!(t - prev, 200, "2:1 of 100 cnt/cycle master");
            prev = t;
        }
        assert_eq!(s.read(GearReadback::Engaged), ChannelValue::Bool(true));
    }

    #[test]
    fn soft_engage_ramps_and_disengage_holds() {
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 1.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 0.1);
        set(&s, GearParam::MasterVel, 100.0);
        set(&s, GearParam::MaxTravel, 1e9);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        let mut last = 0;
        for _ in 0..20 {
            last = e.tick(OP_ENABLED, 0, None);
        }
        assert!(last > 0);
        s.write(GearParam::Engage, &ChannelValue::Bool(false));
        let hold = e.tick(OP_ENABLED, 0, None);
        assert_eq!(hold, last, "disengage holds position");
        assert_eq!(e.tick(OP_ENABLED, 0, None), hold, "keeps holding");
    }

    #[test]
    fn negative_ratio_step_is_inert_not_runaway() {
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 2.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, -0.5);
        set(&s, GearParam::MasterVel, 100.0);
        set(&s, GearParam::MaxTravel, 1e9);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        for _ in 0..20 {
            assert_eq!(e.tick(OP_ENABLED, 0, None), 0, "ratio never ramps");
        }
    }

    #[test]
    fn overtravel_trips_holds_and_needs_engage_drop() {
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 1.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 1.0);
        set(&s, GearParam::MasterVel, 100.0);
        set(&s, GearParam::MaxTravel, 450.0);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        let mut targets = Vec::new();
        for _ in 0..10 {
            targets.push(e.tick(OP_ENABLED, 0, None));
        }
        // Trips once |target| >= 450 (one cycle of overshoot allowed).
        let peak = *targets.iter().max().unwrap();
        assert!(peak >= 450 && peak <= 550, "peak {peak}");
        assert_eq!(*targets.last().unwrap(), peak, "held after trip");
        assert_eq!(s.read(GearReadback::Trip), ChannelValue::Bool(true));
        // Still engaged-requested: stays held (trip persists).
        assert_eq!(e.tick(OP_ENABLED, 0, None), peak);
        // Drop engage → trip clears; re-engage runs again from new origin.
        s.write(GearParam::Engage, &ChannelValue::Bool(false));
        e.tick(OP_ENABLED, 0, None);
        assert_eq!(s.read(GearReadback::Trip), ChannelValue::Bool(false));
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        e.tick(OP_ENABLED, 0, None); // engage tick: at new origin (= held peak)
        let t = e.tick(OP_ENABLED, 0, None);
        assert!(t > peak, "re-engaged and following again");
    }

    #[test]
    fn enable_loss_requires_rearm_before_new_engagement() {
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 1.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 1.0);
        set(&s, GearParam::MasterVel, 100.0);
        set(&s, GearParam::MaxTravel, 1e9);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        e.tick(OP_ENABLED, 0, None); // engage tick: latches origin
        assert!(e.tick(OP_ENABLED, 0, None) > 0, "engaged and moving");
        // Drive drops out (fault / disable); engage force left high.
        e.tick(RDY_ONLY, 500, None);
        // Back to enabled with the stale engage still high: must NOT move.
        let t0 = e.tick(OP_ENABLED, 500, None);
        for _ in 0..10 {
            assert_eq!(e.tick(OP_ENABLED, 500, None), t0, "no auto-restart");
        }
        // Operator re-arms: engage low, then high again → fresh engagement.
        s.write(GearParam::Engage, &ChannelValue::Bool(false));
        e.tick(OP_ENABLED, 500, None);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        e.tick(OP_ENABLED, 500, None);
        assert!(
            e.tick(OP_ENABLED, 500, None) > t0,
            "moves after explicit re-arm"
        );
    }

    #[test]
    fn axis_master_follows_actual() {
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 1.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 1.0);
        set(&s, GearParam::MaxTravel, 1e9);
        // Real bring-up order: shadow while not enabled (s0 arms at the
        // follower's actual), enable with engage low (re-arm), then engage.
        e.tick(RDY_ONLY, 1000, Some(5000)); // shadow: target=1000
        e.tick(OP_ENABLED, 1000, Some(5000)); // enabled, engage low
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        e.tick(OP_ENABLED, 1000, Some(5000)); // engage: m0=5000, s0=1000
        let t = e.tick(OP_ENABLED, 1000, Some(5300));
        assert_eq!(t, 1300, "s0 + 1.0*(master-m0)");
        let t = e.tick(OP_ENABLED, 1000, Some(4000));
        assert_eq!(t, 0, "follows master backwards too");
    }
}
