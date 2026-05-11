use std::sync::{Arc, Mutex};
use std::time::Instant;

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
}

impl AppState {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            start_time: Instant::now(),
            project: Arc::new(Mutex::new(None)),
            program: Arc::new(Mutex::new(None)),
            event_tx,
        }
    }
}
