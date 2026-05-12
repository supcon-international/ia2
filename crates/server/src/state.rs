use std::sync::{Arc, Mutex};
use std::time::Instant;

use iomap_modbus::DemoSlave;
use ironplc_bridge::ProgramHandle;
use project::ProjectStore;
use tokio::sync::broadcast;

use crate::events::AppEvent;

#[derive(Clone)]
pub struct AppState {
    pub start_time: Instant,
    pub project: Arc<Mutex<Option<ProjectStore>>>,
    pub program: Arc<Mutex<Option<ProgramHandle>>>,
    pub event_tx: broadcast::Sender<AppEvent>,
    pub demo_slave: DemoSlave,
    /// The address the in-process demo Modbus slave is listening on
    /// (e.g. "127.0.0.1:5502"). Empty string when the slave is disabled.
    pub demo_modbus_addr: String,
}

impl AppState {
    pub fn new(demo_slave: DemoSlave, demo_modbus_addr: String) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            start_time: Instant::now(),
            project: Arc::new(Mutex::new(None)),
            program: Arc::new(Mutex::new(None)),
            event_tx,
            demo_slave,
            demo_modbus_addr,
        }
    }
}
