//! Consecutive-failure health tracking shared by the fieldbus adapters.
//!
//! Both the Modbus poll task and the EtherCAT cyclic thread need the same
//! tiny state machine: count consecutive transfer failures, flip a shared
//! "unhealthy" flag once a threshold is crossed (logging ERROR exactly
//! once, not per cycle), and clear it on the first success (logging INFO
//! once). The tracker owns the counting; the adapter owns the logging —
//! the returned [`HealthTransition`] tells it when a log line is due.
//!
//! The flag itself is an `Arc<AtomicBool>` so the adapter façade can
//! answer `is_healthy()` from another task/thread without locking.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// What a `record_*` call changed — drives one-shot logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthTransition {
    /// No edge: still healthy, or still unhealthy.
    Unchanged,
    /// This failure crossed the threshold — log ERROR now (once).
    BecameUnhealthy,
    /// First success after being unhealthy — log INFO now (once).
    Recovered,
}

/// Counts consecutive failures against a threshold and mirrors the
/// resulting health into a shared [`AtomicBool`] (starts healthy).
#[derive(Debug)]
pub struct HealthTracker {
    threshold: u32,
    consecutive_failures: u32,
    flag: Arc<AtomicBool>,
}

impl HealthTracker {
    /// New tracker that flips unhealthy after `threshold` consecutive
    /// failures (a threshold of 0 is treated as 1).
    pub fn new(threshold: u32) -> Self {
        Self::with_flag(threshold, Arc::new(AtomicBool::new(true)))
    }

    /// As [`HealthTracker::new`], but mirroring into a caller-supplied
    /// flag (e.g. one already shared with a device façade). The flag is
    /// reset to healthy.
    pub fn with_flag(threshold: u32, flag: Arc<AtomicBool>) -> Self {
        flag.store(true, Ordering::Relaxed);
        Self {
            threshold: threshold.max(1),
            consecutive_failures: 0,
            flag,
        }
    }

    /// Clone of the shared flag, for `is_healthy()` on the device façade.
    pub fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }

    pub fn is_healthy(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Failures seen since the last success.
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Record one failed transfer. Returns `BecameUnhealthy` exactly when
    /// the threshold is crossed (the `threshold`-th consecutive failure).
    pub fn record_failure(&mut self) -> HealthTransition {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures == self.threshold {
            self.flag.store(false, Ordering::Relaxed);
            HealthTransition::BecameUnhealthy
        } else {
            HealthTransition::Unchanged
        }
    }

    /// Record one successful transfer. Returns `Recovered` exactly when
    /// this success ends an unhealthy stretch.
    pub fn record_success(&mut self) -> HealthTransition {
        let was_unhealthy = !self.is_healthy();
        self.consecutive_failures = 0;
        if was_unhealthy {
            self.flag.store(true, Ordering::Relaxed);
            HealthTransition::Recovered
        } else {
            HealthTransition::Unchanged
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_healthy_and_stays_healthy_below_threshold() {
        let mut t = HealthTracker::new(3);
        assert!(t.is_healthy());
        assert_eq!(t.record_failure(), HealthTransition::Unchanged);
        assert_eq!(t.record_failure(), HealthTransition::Unchanged);
        assert!(t.is_healthy(), "two failures < threshold of three");
        assert_eq!(t.consecutive_failures(), 2);
    }

    #[test]
    fn crosses_threshold_exactly_once() {
        let mut t = HealthTracker::new(3);
        t.record_failure();
        t.record_failure();
        assert_eq!(
            t.record_failure(),
            HealthTransition::BecameUnhealthy,
            "third consecutive failure must announce the edge"
        );
        assert!(!t.is_healthy());
        // Further failures keep it unhealthy but never re-announce —
        // that's what keeps the ERROR log to one line per outage.
        assert_eq!(t.record_failure(), HealthTransition::Unchanged);
        assert_eq!(t.record_failure(), HealthTransition::Unchanged);
        assert!(!t.is_healthy());
    }

    #[test]
    fn success_resets_counter_before_threshold_without_announcing() {
        let mut t = HealthTracker::new(3);
        t.record_failure();
        t.record_failure();
        assert_eq!(t.record_success(), HealthTransition::Unchanged);
        assert_eq!(t.consecutive_failures(), 0);
        // The streak starts over: two more failures still don't trip it.
        t.record_failure();
        t.record_failure();
        assert!(t.is_healthy());
    }

    #[test]
    fn recovers_exactly_once_after_unhealthy() {
        let mut t = HealthTracker::new(2);
        t.record_failure();
        t.record_failure();
        assert!(!t.is_healthy());
        assert_eq!(t.record_success(), HealthTransition::Recovered);
        assert!(t.is_healthy());
        assert_eq!(
            t.record_success(),
            HealthTransition::Unchanged,
            "recovery announces once, not per success"
        );
    }

    #[test]
    fn shared_flag_is_visible_through_clones() {
        let mut t = HealthTracker::new(1);
        let flag = t.flag();
        assert!(flag.load(Ordering::Relaxed));
        t.record_failure();
        assert!(!flag.load(Ordering::Relaxed), "façade sees the flip");
        t.record_success();
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn zero_threshold_behaves_as_one() {
        let mut t = HealthTracker::new(0);
        assert_eq!(t.record_failure(), HealthTransition::BecameUnhealthy);
    }

    #[test]
    fn with_flag_resets_a_stale_flag_to_healthy() {
        let stale = Arc::new(AtomicBool::new(false));
        let t = HealthTracker::with_flag(3, stale.clone());
        assert!(t.is_healthy());
        assert!(stale.load(Ordering::Relaxed));
    }
}
