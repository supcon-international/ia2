//! Spawns a thread that runs the ironplc VM scan loop and broadcasts variable
//! snapshots to subscribers via `tokio::sync::broadcast`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ironplc_container::Container;
use ironplc_container::VarIndex;
use ironplc_container::debug_format::{build_var_debug_map, format_variable_value};
use ironplc_vm::{Vm, VmBuffers};
use serde::Serialize;
use tokio::sync::broadcast;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct VarValue {
    pub name: String,
    pub type_name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct VarSnapshot {
    pub timestamp_us: u64,
    pub scan_count: u64,
    pub vars: Vec<VarValue>,
}

pub struct ProgramHandle {
    stop: Arc<AtomicBool>,
    snapshot_tx: broadcast::Sender<VarSnapshot>,
}

impl ProgramHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<VarSnapshot> {
        self.snapshot_tx.subscribe()
    }
}

/// Take ownership of a compiled `Container`, spawn a VM thread, and return a
/// handle for stopping it and subscribing to variable snapshots.
pub fn spawn(container: Container) -> ProgramHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let (snapshot_tx, _) = broadcast::channel(64);

    let stop_clone = stop.clone();
    let snapshot_tx_clone = snapshot_tx.clone();

    std::thread::spawn(move || {
        run_loop(container, stop_clone, snapshot_tx_clone);
    });

    ProgramHandle { stop, snapshot_tx }
}

fn run_loop(
    container: Container,
    stop: Arc<AtomicBool>,
    snapshot_tx: broadcast::Sender<VarSnapshot>,
) {
    let mut bufs = VmBuffers::from_container(&container);
    let mut running = match Vm::new().load(&container, &mut bufs).start() {
        Ok(r) => r,
        Err(ctx) => {
            tracing::error!(?ctx.trap, "vm failed to start");
            return;
        }
    };

    let debug_map = build_var_debug_map(&container);
    let start = Instant::now();
    let mut last_snapshot = Instant::now() - Duration::from_secs(1);
    let mut scan_count: u64 = 0;

    loop {
        if stop.load(Ordering::Relaxed) {
            running.request_stop();
        }
        if running.stop_requested() {
            break;
        }

        let now_us = start.elapsed().as_micros() as u64;
        if let Err(ctx) = running.run_round(now_us) {
            tracing::error!(?ctx.trap, "vm trap during run_round");
            break;
        }
        scan_count += 1;

        // Snapshot variables ~10 Hz.
        if last_snapshot.elapsed() >= Duration::from_millis(100) {
            let num_vars = running.num_variables();
            let mut vars = Vec::with_capacity(num_vars as usize);
            for i in 0..num_vars {
                let raw = match running.read_variable_raw(VarIndex::new(i)) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let (name, type_name, tag) = match debug_map.get(&i) {
                    Some(info) => (
                        info.name.clone(),
                        info.type_name.clone(),
                        info.iec_type_tag,
                    ),
                    None => (format!("var[{i}]"), String::new(), 0),
                };
                vars.push(VarValue {
                    name,
                    type_name,
                    value: format_variable_value(raw, tag),
                });
            }
            let snapshot = VarSnapshot {
                timestamp_us: now_us,
                scan_count,
                vars,
            };
            // Ignore SendError when no subscribers are attached.
            let _ = snapshot_tx.send(snapshot);
            last_snapshot = Instant::now();
        }

        // Wait until the next cyclic task is due, or yield briefly for
        // freewheeling programs.
        if let Some(due_us) = running.next_due_us() {
            let now_us = start.elapsed().as_micros() as u64;
            let sleep_us = due_us.saturating_sub(now_us);
            if sleep_us > 0 {
                std::thread::sleep(Duration::from_micros(sleep_us));
            }
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    let _ = running.stop();
}
