use ironplc_bridge::VarSnapshot;
use serde::Serialize;
use ts_rs::TS;

/// Server-pushed event delivered over SSE (`GET /api/events`).
///
/// Wire form is adjacently tagged JSON, e.g.
/// `{"type":"snapshot","data":{...VarSnapshot...}}` or `{"type":"started"}`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[allow(dead_code)]
pub enum AppEvent {
    Snapshot(VarSnapshot),
    Started,
    Stopped,
    Error(String),
}
