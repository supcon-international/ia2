//! The trait an external fieldbus adapter (Modbus, EtherCAT, …) implements
//! so the ironplc VM scan loop can read inputs before `run_round` and write
//! outputs after.
//!
//! Lives in its own crate so concrete adapters (`iomap-modbus`,
//! `iomap-ethercat`) and the ironplc-bridge runtime can all depend on it
//! without forming a cycle.

use async_trait::async_trait;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum ChannelValue {
    Bool(bool),
    U16(u16),
    I32(i32),
    /// IEEE-754 single — analog values (OPC UA Float tags, EtherCAT REAL
    /// PDOs, scaled 4-20 mA). Carried as a real float so fractional
    /// process values (12.7 m³/h) survive the trip to a REAL PLC var.
    Real(f32),
}

impl ChannelValue {
    /// Coerce to a *numeric* i32 (Real is truncated). Display/legacy
    /// lane — for feeding the VM use `to_vm_bits`, which respects the
    /// target variable's type.
    pub fn to_i32(self) -> i32 {
        match self {
            Self::Bool(b) => b as i32,
            Self::U16(v) => v as i32,
            Self::I32(v) => v,
            Self::Real(f) => f as i32,
        }
    }

    /// Numeric f32 view (integers convert by value).
    pub fn to_f32(self) -> f32 {
        match self {
            Self::Bool(b) => b as i32 as f32,
            Self::U16(v) => v as f32,
            Self::I32(v) => v as f32,
            Self::Real(f) => f,
        }
    }

    /// Encode for `write_variable` on the ironplc VM, which takes an i32
    /// whose meaning depends on the *target variable's* IEC type: REAL
    /// variables reinterpret the i32 as IEEE-754 bits, integer variables
    /// take it by value. Mismatched pairs convert numerically first, so
    /// an integer channel bound to a REAL var (or vice versa) does the
    /// right thing instead of smuggling a bit pattern.
    pub fn to_vm_bits(self, var_is_real: bool) -> i32 {
        if var_is_real {
            self.to_f32().to_bits() as i32
        } else {
            self.to_i32()
        }
    }
}

#[derive(Debug, Error)]
pub enum IoError {
    #[error("unknown channel '{0}'")]
    UnknownChannel(String),
    #[error("type mismatch: channel '{channel}' cannot accept {value:?}")]
    TypeMismatch {
        channel: String,
        value: ChannelValue,
    },
    #[error("connect: {0}")]
    Connect(String),
    #[error("transport: {0}")]
    Transport(String),
}

/// A fieldbus device — read/write a logical channel by name. Implementations
/// own their connection / runtime / cache.
#[async_trait]
pub trait IoDevice: Send {
    fn name(&self) -> &str;
    async fn read_channel(&mut self, channel: &str) -> Result<ChannelValue, IoError>;
    async fn write_channel(&mut self, channel: &str, value: ChannelValue) -> Result<(), IoError>;

    /// Drive all writable outputs to a known-safe state (zero / "off").
    ///
    /// Called by the bridge scan loop in three situations:
    ///   1. **Panic** during a scan round — the run-loop catches the
    ///      unwind and triggers failsafe before the thread exits.
    ///   2. **Consecutive scan-deadline overruns** above a threshold —
    ///      "the simulation is no longer real-time, freeze the plant".
    ///   3. **Graceful shutdown** — explicit stop request.
    ///
    /// Industrial PLCs do this via a hardware watchdog; we don't have
    /// hardware here, so the bridge orchestrates the equivalent in
    /// software. Implementations should:
    ///   - Write a zero/safe value to every output channel they know
    ///     about. Read-only channels are skipped.
    ///   - Best-effort: a transport error on one channel should not
    ///     stop the loop from trying the rest. Return the first error
    ///     so the caller can log.
    ///
    /// Default impl is a no-op so devices that genuinely have no
    /// writable surface (e.g. a read-only sensor adapter) need no
    /// extra code.
    async fn enter_failsafe(&mut self) -> Result<(), IoError> {
        Ok(())
    }

    /// Wind the device down for a clean process exit. Called once by the
    /// bridge on graceful shutdown, AFTER `enter_failsafe`, so an
    /// implementation can flush its now-safe outputs and join any
    /// background I/O thread it owns before the process goes away.
    ///
    /// This is what lets the in-runtime failsafe actually reach the wire:
    /// e.g. the EtherCAT adapter runs its cyclic exchange on a dedicated
    /// thread, so it signals + joins that thread here to guarantee the
    /// zeroed outputs (controlword = 0) are transmitted before teardown,
    /// rather than relying on the drive's own watchdog after the master
    /// is killed.
    ///
    /// Implementations MUST be bounded — the runtime only has a few
    /// seconds before the service supervisor force-kills it. Default impl
    /// is a no-op for devices with no background work to wind down (e.g.
    /// sim, or Modbus whose `enter_failsafe` already wrote synchronously).
    async fn shutdown(&mut self) -> Result<(), IoError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_bits_for_real_vars_are_ieee754() {
        // Real → REAL var: bit pattern.
        assert_eq!(
            ChannelValue::Real(12.7).to_vm_bits(true),
            12.7f32.to_bits() as i32
        );
        // Integer channel → REAL var: convert by value first.
        assert_eq!(
            ChannelValue::U16(42).to_vm_bits(true),
            42.0f32.to_bits() as i32
        );
        // Real → integer var: numeric truncation, not bits.
        assert_eq!(ChannelValue::Real(12.7).to_vm_bits(false), 12);
        // Integer → integer: identity.
        assert_eq!(ChannelValue::I32(-7).to_vm_bits(false), -7);
    }
}
