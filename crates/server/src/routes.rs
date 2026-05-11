use std::convert::Infallible;
use std::time::Duration;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::stream::Stream;
use serde::Serialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use ts_rs::TS;

use crate::events::AppEvent;
use crate::sample::{SAMPLE_NAME, SAMPLE_SOURCE};
use crate::state::AppState;

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct HealthStatus {
    pub status: String,
    pub uptime_secs: u64,
    pub program_running: bool,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ProgramInfo {
    pub name: String,
    pub source: String,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct RunResponse {
    pub ok: bool,
}

pub async fn health(State(state): State<AppState>) -> Json<HealthStatus> {
    let elapsed = state.start_time.elapsed().as_secs();
    let running = state.program.lock().expect("program mutex poisoned").is_some();
    Json(HealthStatus {
        status: "ok".into(),
        uptime_secs: elapsed,
        program_running: running,
    })
}

pub async fn program() -> Json<ProgramInfo> {
    Json(ProgramInfo {
        name: SAMPLE_NAME.into(),
        source: SAMPLE_SOURCE.into(),
    })
}

pub async fn run(
    State(state): State<AppState>,
    body: String,
) -> Result<Json<RunResponse>, (StatusCode, String)> {
    // Empty body → fall back to the bundled sample so `curl -X POST /api/run`
    // (no body) still works as a quick demo.
    let source = if body.trim().is_empty() {
        SAMPLE_SOURCE
    } else {
        body.as_str()
    };
    let container = ironplc_bridge::compile(source)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    {
        let mut guard = state.program.lock().expect("program mutex poisoned");
        if let Some(old) = guard.take() {
            old.stop();
        }
    }

    let handle = ironplc_bridge::spawn(container);
    let mut rx = handle.subscribe();
    let event_tx = state.event_tx.clone();

    tokio::spawn(async move {
        // Forward snapshots even when no SSE clients are subscribed yet — they
        // may connect later. Only exit when the bridge channel itself closes.
        while let Ok(snap) = rx.recv().await {
            let _ = event_tx.send(AppEvent::Snapshot(snap));
        }
    });

    state
        .program
        .lock()
        .expect("program mutex poisoned")
        .replace(handle);
    let _ = state.event_tx.send(AppEvent::Started);

    Ok(Json(RunResponse { ok: true }))
}

pub async fn stop(State(state): State<AppState>) -> Json<RunResponse> {
    if let Some(handle) = state
        .program
        .lock()
        .expect("program mutex poisoned")
        .take()
    {
        handle.stop();
    }
    let _ = state.event_tx.send(AppEvent::Stopped);
    Json(RunResponse { ok: true })
}

pub async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(event) => Event::default().json_data(&event).ok().map(Ok),
        // Lagged subscribers: skip the missed event rather than disconnect.
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
