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
        if let GearParam::Engage = param {
            self.engage.store(as_f64 != 0.0, Ordering::Relaxed);
            return;
        }
        // Reject non-finite numeric params at the door: a slow-plane NaN/inf
        // (e.g. an ST `0.0/0.0` while composing a ratio) must never reach the
        // fast-plane law, where it would poison the target and silently
        // bypass the max_travel trip. Dropping the write keeps the last good
        // value in force.
        if !as_f64.is_finite() {
            return;
        }
        match param {
            GearParam::Engage => unreachable!(),
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
    /// Continuous, wrap-free master position (counts). For a virtual master
    /// this is an accumulator advanced by `master_vel`; for an axis master
    /// it integrates the per-cycle *wrapping* i32 delta of the master's
    /// actual_position, so the ±2^31 encoder wrap never becomes a full-scale
    /// jump in `(master - m0)`. `None` until the first tick primes it.
    master_pos: f64,
    /// Previous raw i32 master actual, for the wrapping-delta integration.
    prev_master_raw: Option<i32>,
    /// Current output target (counts). Shadow / gear law / hold all land
    /// here; it is the single source of what goes on the wire.
    target: f64,
    was_engaged: bool,
    m0: f64,
    s0: f64,
    ratio: f64,
    /// Gear parameters latched at the engage edge (the documented contract:
    /// mid-run edits are inert until re-engage). `ratio_step` stays live —
    /// it only sets the soft-engage ramp speed.
    ratio_num_l: f64,
    ratio_den_l: f64,
    phase_l: f64,
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
            master_pos: 0.0,
            prev_master_raw: None,
            target: 0.0,
            was_engaged: false,
            m0: 0.0,
            s0: 0.0,
            ratio: 0.0,
            ratio_num_l: 0.0,
            ratio_den_l: 1.0,
            phase_l: 0.0,
            trip: false,
            need_rearm: false,
        }
    }

    /// One bus cycle. `master_actual` is `Some` for an axis master (read by
    /// the caller from that axis's input PDI) and `None` for virtual.
    /// `bus_ok` is `false` when the *previous* exchange failed, so the inputs
    /// are stale — the engine then freezes (no master advance, target held)
    /// and recovery applies no accumulated step. Returns the target_position
    /// to write into the follower's output PDI.
    pub fn tick(
        &mut self,
        follower_sw: u16,
        follower_actual: i32,
        master_actual: Option<i32>,
        bus_ok: bool,
    ) -> i32 {
        // Stale-input cycle (previous tx_rx failed): hold the target and
        // FORGET the master baseline. During an outage `sd.inputs_raw()`
        // keeps returning the pre-outage value, so re-priming to it would be
        // a no-op — and then the first fresh reading after recovery would
        // integrate the ENTIRE outage displacement in one cycle (a catch-up
        // jerk into the follower). Dropping the baseline instead makes the
        // first good cycle re-prime with a ZERO delta: the gear resyncs to
        // the master's current position and continues, accepting a one-time
        // phase slip rather than commanding the whole gap. (A virtual master
        // has no baseline to keep — it's simply frozen while `bus_ok` is
        // false.)
        if !bus_ok {
            self.prev_master_raw = None;
            return self.emit();
        }

        let engage_in = self.shared.engage.load(Ordering::Relaxed);
        // Slow-plane values are sanitized to finite at ingress (see
        // GearShared::write), so no NaN/inf can reach the law here.
        let ratio_step = GearShared::load_f64(&self.shared.ratio_step).max(0.0);
        let master_vel = GearShared::load_f64(&self.shared.master_vel);
        let max_travel = GearShared::load_f64(&self.shared.max_travel);

        // Advance the continuous, wrap-free master position.
        match master_actual {
            // Axis master: integrate the WRAPPING i32 delta so the encoder's
            // ±2^31 rollover contributes a small step, not a full-scale jump.
            Some(raw) => {
                if let Some(prev) = self.prev_master_raw {
                    self.master_pos += raw.wrapping_sub(prev) as f64;
                }
                self.prev_master_raw = Some(raw);
            }
            // Virtual master: software accumulator, advances every tick.
            None => self.master_pos += master_vel,
        }

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
                // Engage edge: latch origins AND gear params (documented
                // contract — mid-run ratio/phase edits are inert until
                // re-engage). ratio_step stays live (ramp speed only).
                self.m0 = self.master_pos;
                self.s0 = self.target;
                self.ratio = 0.0;
                self.ratio_num_l = GearShared::load_f64(&self.shared.ratio_num);
                self.ratio_den_l = GearShared::load_f64(&self.shared.ratio_den);
                self.phase_l = GearShared::load_f64(&self.shared.phase_ofs);
            }
            self.was_engaged = engage;
            if engage {
                let tgt_ratio = if self.ratio_den_l == 0.0 {
                    0.0
                } else {
                    self.ratio_num_l / self.ratio_den_l
                };
                let d = (tgt_ratio - self.ratio).clamp(-ratio_step, ratio_step);
                self.ratio += d;
                let raw_target = self.s0 + self.ratio * (self.master_pos - self.m0) + self.phase_l;
                // Hard travel bound: clamp the commanded excursion to
                // ±max_travel about the engage origin, so max_travel is a
                // true limit on what reaches the wire — not just a
                // fire-after-the-fact trip. A wild ratio typo or any other
                // over-range demand is capped to one max_travel excursion,
                // and the trip latches to hold there until engage drops.
                self.target = raw_target.clamp(self.s0 - max_travel, self.s0 + max_travel);
                if (raw_target - self.s0).abs() >= max_travel {
                    self.trip = true;
                }
            }
            // Disengaged / tripped / re-arm pending: hold self.target.
        }

        self.emit()
    }

    /// Publish engine state and encode the held target for the wire.
    fn emit(&self) -> i32 {
        self.shared
            .engaged_fb
            .store(self.was_engaged, Ordering::Relaxed);
        self.shared.trip_fb.store(self.trip, Ordering::Relaxed);

        // Saturate rather than wrap at the i32 edge (±256 motor revs on a
        // 23-bit encoder). Long unidirectional runs remain a known limit.
        // `target` is always finite: params are finite by ingress guard and
        // clamped by max_travel, so round()/clamp can't see NaN here.
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

/// Connect-time check that every gear channel name is unique — across all
/// gears and within each gear — and doesn't shadow a PDO channel. Without
/// this, `build`'s HashMap inserts silently overwrite a duplicate, so a
/// second gear axis (or a fat-fingered channel rename) would misroute
/// parameters to the wrong engine with no error. Returns the first
/// collision found. Called by both the real and sim connect paths.
pub fn validate_channels(
    gears: &[EthercatGear],
    pdo_names: &std::collections::HashSet<&str>,
) -> Result<(), String> {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for g in gears {
        for name in [
            &g.engage_channel,
            &g.ratio_num_channel,
            &g.ratio_den_channel,
            &g.ratio_step_channel,
            &g.phase_channel,
            &g.master_vel_channel,
            &g.max_travel_channel,
            &g.engaged_channel,
            &g.trip_channel,
        ] {
            if pdo_names.contains(name.as_str()) {
                return Err(format!(
                    "gear channel '{name}' collides with a PDO channel name"
                ));
            }
            if !seen.insert(name.as_str()) {
                return Err(format!(
                    "gear channel '{name}' is used by more than one gear axis or parameter"
                ));
            }
        }
    }
    Ok(())
}

/// Build the engines + routing tables from the device config. Returns the
/// engines for the cyclic worker and the routing for the device facade.
/// Assumes [`validate_channels`] has already passed (unique names).
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
        assert_eq!(e.tick(RDY_ONLY, 12345, None, true), 12345);
        assert_eq!(e.tick(RDY_ONLY, -777, None, true), -777);
        // Enable: target latched (holds), no jump.
        assert_eq!(e.tick(OP_ENABLED, -777, None, true), -777);
        assert_eq!(
            e.tick(OP_ENABLED, -700, None, true),
            -777,
            "hold, not follow"
        );
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
            assert_eq!(
                e.tick(OP_ENABLED, 0, None, true),
                0,
                "locked while max_travel=0"
            );
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
        let t1 = e.tick(OP_ENABLED, 0, None, true);
        let mut prev = t1;
        for _ in 0..50 {
            let t = e.tick(OP_ENABLED, 0, None, true);
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
            last = e.tick(OP_ENABLED, 0, None, true);
        }
        assert!(last > 0);
        s.write(GearParam::Engage, &ChannelValue::Bool(false));
        let hold = e.tick(OP_ENABLED, 0, None, true);
        assert_eq!(hold, last, "disengage holds position");
        assert_eq!(e.tick(OP_ENABLED, 0, None, true), hold, "keeps holding");
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
            assert_eq!(e.tick(OP_ENABLED, 0, None, true), 0, "ratio never ramps");
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
            targets.push(e.tick(OP_ENABLED, 0, None, true));
        }
        // Trips once |target| >= 450 (one cycle of overshoot allowed).
        let peak = *targets.iter().max().unwrap();
        assert!(peak >= 450 && peak <= 550, "peak {peak}");
        assert_eq!(*targets.last().unwrap(), peak, "held after trip");
        assert_eq!(s.read(GearReadback::Trip), ChannelValue::Bool(true));
        // Still engaged-requested: stays held (trip persists).
        assert_eq!(e.tick(OP_ENABLED, 0, None, true), peak);
        // Drop engage → trip clears; re-engage runs again from new origin.
        s.write(GearParam::Engage, &ChannelValue::Bool(false));
        e.tick(OP_ENABLED, 0, None, true);
        assert_eq!(s.read(GearReadback::Trip), ChannelValue::Bool(false));
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        e.tick(OP_ENABLED, 0, None, true); // engage tick: at new origin (= held peak)
        let t = e.tick(OP_ENABLED, 0, None, true);
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
        e.tick(OP_ENABLED, 0, None, true); // engage tick: latches origin
        assert!(e.tick(OP_ENABLED, 0, None, true) > 0, "engaged and moving");
        // Drive drops out (fault / disable); engage force left high.
        e.tick(RDY_ONLY, 500, None, true);
        // Back to enabled with the stale engage still high: must NOT move.
        let t0 = e.tick(OP_ENABLED, 500, None, true);
        for _ in 0..10 {
            assert_eq!(e.tick(OP_ENABLED, 500, None, true), t0, "no auto-restart");
        }
        // Operator re-arms: engage low, then high again → fresh engagement.
        s.write(GearParam::Engage, &ChannelValue::Bool(false));
        e.tick(OP_ENABLED, 500, None, true);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        e.tick(OP_ENABLED, 500, None, true);
        assert!(
            e.tick(OP_ENABLED, 500, None, true) > t0,
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
        e.tick(RDY_ONLY, 1000, Some(5000), true); // shadow: target=1000
        e.tick(OP_ENABLED, 1000, Some(5000), true); // enabled, engage low
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        e.tick(OP_ENABLED, 1000, Some(5000), true); // engage: m0=5000, s0=1000
        let t = e.tick(OP_ENABLED, 1000, Some(5300), true);
        assert_eq!(t, 1300, "s0 + 1.0*(master-m0)");
        let t = e.tick(OP_ENABLED, 1000, Some(4000), true);
        assert_eq!(t, 0, "follows master backwards too");
    }

    #[test]
    fn axis_master_i32_wrap_is_a_small_step_not_full_scale() {
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 1.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 1.0);
        set(&s, GearParam::MaxTravel, 1e9);
        // Engage with the master near the i32 max.
        let near_max = i32::MAX - 100;
        e.tick(RDY_ONLY, 0, Some(near_max), true);
        e.tick(OP_ENABLED, 0, Some(near_max), true);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        let base = e.tick(OP_ENABLED, 0, Some(near_max), true); // engage edge
                                                                // Advance 200 counts, which wraps i32 (MAX-100 + 200 -> MIN+99).
        let wrapped = near_max.wrapping_add(200);
        assert!(wrapped < 0, "precondition: the master actually wrapped");
        let t = e.tick(OP_ENABLED, 0, Some(wrapped), true);
        assert_eq!(t - base, 200, "wrap contributes a +200 step, not ~-4.3e9");
    }

    #[test]
    fn nan_param_is_rejected_at_ingress_and_target_stays_finite() {
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 2.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 1.0);
        set(&s, GearParam::MasterVel, 100.0);
        set(&s, GearParam::MaxTravel, 1e9);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        e.tick(OP_ENABLED, 0, None, true);
        let good = e.tick(OP_ENABLED, 0, None, true);
        // Slow plane writes NaN (e.g. ST 0.0/0.0) to ratio_den; must be dropped.
        s.write(GearParam::RatioDen, &ChannelValue::F64(f64::NAN));
        s.write(GearParam::RatioNum, &ChannelValue::F64(f64::INFINITY));
        assert_eq!(
            s.read(GearReadback::RatioDen),
            ChannelValue::F64(1.0),
            "NaN write dropped, last good value kept"
        );
        // Engine keeps producing finite, monotonic targets (2:1 unchanged).
        let t1 = e.tick(OP_ENABLED, 0, None, true);
        let t2 = e.tick(OP_ENABLED, 0, None, true);
        assert_eq!(t1 - good, 200);
        assert_eq!(t2 - t1, 200, "still 2:1, not poisoned to 0");
    }

    #[test]
    fn mid_run_ratio_and_phase_edits_are_inert_until_reengage() {
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 1.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 10.0); // hard engage: full ratio in 1 tick
        set(&s, GearParam::MasterVel, 100.0);
        set(&s, GearParam::MaxTravel, 1e9);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        e.tick(OP_ENABLED, 0, None, true); // engage: latch 1:1, phase 0
        let a = e.tick(OP_ENABLED, 0, None, true);
        let b = e.tick(OP_ENABLED, 0, None, true);
        assert_eq!(b - a, 100, "1:1");
        // Mid-run edits: must NOT take effect while engaged.
        set(&s, GearParam::RatioNum, 5.0);
        set(&s, GearParam::PhaseOfs, 1_000_000.0);
        let c = e.tick(OP_ENABLED, 0, None, true);
        let d = e.tick(OP_ENABLED, 0, None, true);
        assert_eq!(d - c, 100, "still 1:1, no phase step — edits inert");
        // Re-engage picks up the new params.
        s.write(GearParam::Engage, &ChannelValue::Bool(false));
        e.tick(OP_ENABLED, 0, None, true);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        e.tick(OP_ENABLED, 0, None, true); // engage: latch 5:1
        let p = e.tick(OP_ENABLED, 0, None, true);
        let q = e.tick(OP_ENABLED, 0, None, true);
        assert_eq!(q - p, 500, "now 5:1 after re-engage");
    }

    #[test]
    fn max_travel_hard_clamps_the_commanded_excursion() {
        let (mut e, s) = engine_virtual();
        // Absurd ratio typo: 1000:1. Without the hard clamp this steps ~1e5
        // counts in the first engaged cycle.
        set(&s, GearParam::RatioNum, 1000.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 1000.0);
        set(&s, GearParam::MasterVel, 100.0);
        set(&s, GearParam::MaxTravel, 450.0);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        let base = e.tick(OP_ENABLED, 0, None, true); // engage edge, s0 here
        let mut peak_excursion = 0;
        for _ in 0..8 {
            let t = e.tick(OP_ENABLED, 0, None, true);
            peak_excursion = peak_excursion.max((t - base).abs());
        }
        assert!(
            peak_excursion <= 450,
            "excursion {peak_excursion} must never exceed max_travel, even for a wild ratio"
        );
        assert_eq!(s.read(GearReadback::Trip), ChannelValue::Bool(true));
    }

    #[test]
    fn stale_bus_cycle_freezes_master_and_holds_target() {
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 1.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 1.0);
        set(&s, GearParam::MasterVel, 100.0);
        set(&s, GearParam::MaxTravel, 1e9);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        e.tick(OP_ENABLED, 0, None, true);
        let before = e.tick(OP_ENABLED, 0, None, true);
        // 5 stale cycles (bus down): master must NOT integrate, target holds.
        for _ in 0..5 {
            assert_eq!(
                e.tick(OP_ENABLED, 0, None, false),
                before,
                "held during outage"
            );
        }
        // Recovery: exactly one cycle of advance, no 5-cycle catch-up.
        let after = e.tick(OP_ENABLED, 0, None, true);
        assert_eq!(
            after - before,
            100,
            "single-cycle step on recovery, not 600"
        );
    }

    #[test]
    fn stale_bus_axis_master_resyncs_without_catchup_step() {
        // The dangerous case: an AXIS master keeps physically turning during
        // a bus outage while its PDI image is frozen at the pre-outage value.
        // On recovery the engine must resync to the master's new position
        // with ZERO commanded step, not integrate the whole outage gap.
        let (mut e, s) = engine_virtual();
        set(&s, GearParam::RatioNum, 1.0);
        set(&s, GearParam::RatioDen, 1.0);
        set(&s, GearParam::RatioStep, 1.0);
        set(&s, GearParam::MaxTravel, 1e9);
        e.tick(RDY_ONLY, 0, Some(1000), true);
        e.tick(OP_ENABLED, 0, Some(1000), true);
        s.write(GearParam::Engage, &ChannelValue::Bool(true));
        let base = e.tick(OP_ENABLED, 0, Some(1000), true); // engage edge
                                                            // Outage: 5 stale cycles, PDI frozen at the last-good raw (1000).
        for _ in 0..5 {
            assert_eq!(e.tick(OP_ENABLED, 0, Some(1000), false), base, "held");
        }
        // Recovery: fresh raw is 1600 (master moved 600 during the outage).
        let after = e.tick(OP_ENABLED, 0, Some(1600), true);
        assert_eq!(
            after, base,
            "resync with zero step, NOT a +600 catch-up jerk"
        );
        // And it follows normally from the new baseline afterwards.
        let next = e.tick(OP_ENABLED, 0, Some(1650), true);
        assert_eq!(next - after, 50, "resumes 1:1 from the resynced position");
    }

    #[test]
    fn validate_channels_catches_gear_collisions() {
        use std::collections::HashSet;
        let mut a = EthercatGear {
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
        let empty: HashSet<&str> = HashSet::new();
        assert!(validate_channels(std::slice::from_ref(&a), &empty).is_ok());
        // PDO shadow.
        let pdo: HashSet<&str> = ["ratio_num"].into_iter().collect();
        assert!(validate_channels(std::slice::from_ref(&a), &pdo).is_err());
        // Two default-named gears collide.
        let b = a.clone();
        assert!(validate_channels(&[a.clone(), b], &empty).is_err());
        // Intra-gear collision.
        a.phase_channel = "ratio_num".into();
        assert!(validate_channels(std::slice::from_ref(&a), &empty).is_err());
    }
}
