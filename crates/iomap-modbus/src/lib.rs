//! Modbus TCP IoDevice adapter + a small in-memory demo slave (so users
//! can wire the IDE up end-to-end without external hardware).

mod client;
mod slave;

pub use client::ModbusDevice;
pub use slave::{DemoSlave, run_demo_slave};
