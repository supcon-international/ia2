//! Northbound MQTT publisher — edge runtime → plant platform (supOS /
//! Tier0).
//!
//! MQTT only, by design: the platform ingests MQTT natively, and the
//! northbound link is a *data* path. Southbound control stays on the
//! device layer (OPC UA → DCS, EtherCAT, Modbus).
//!
//! Topics (prefix defaults to `ia2/<project>`):
//!   - `<prefix>/status`    retained `online` / `offline`, with an MQTT
//!     Last-Will so the platform sees crashes too, not just clean exits.
//!   - `<prefix>/snapshot`  every `publish_interval_ms`:
//!     `{"ts_us":…,"scan":…,"values":{"FT0202":12.7,"alarm_h":true,…}}`
//!     Values are decoded JSON numbers/bools (not the IDE's display
//!     strings), so platform-side JSON-path mapping is one hop.
//!   - `<prefix>/write`     subscribed only when `allow_write = true`:
//!     `{"name":"sp_flow","value":12.5}` → one-shot variable write,
//!     same semantics as POST /write (the program can overwrite next
//!     scan; use program logic to latch setpoints).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironplc_bridge::{ProgramHandle, VarSnapshot};
use project::MqttNorthbound;
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};

pub struct NorthboundCtx {
    pub config: MqttNorthbound,
    pub project_name: String,
    pub latest: Arc<Mutex<Option<VarSnapshot>>>,
    pub handle: ProgramHandle,
}

/// Spawn the northbound task. Returns immediately; the task owns the
/// MQTT connection (rumqttc's event loop reconnects with backoff on its
/// own, so a broker restart just pauses publishing).
pub fn spawn(ctx: NorthboundCtx) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(ctx))
}

fn qos_of(q: u8) -> QoS {
    match q {
        0 => QoS::AtMostOnce,
        _ => QoS::AtLeastOnce,
    }
}

async fn run(ctx: NorthboundCtx) {
    let cfg = &ctx.config;
    let client_id = if cfg.client_id.is_empty() {
        format!("ia2-{}", ctx.project_name)
    } else {
        cfg.client_id.clone()
    };
    let prefix = if cfg.topic_prefix.is_empty() {
        format!("ia2/{}", ctx.project_name)
    } else {
        cfg.topic_prefix.trim_end_matches('/').to_string()
    };
    let qos = qos_of(cfg.qos);

    let mut options = MqttOptions::new(client_id, &cfg.broker_host, cfg.broker_port);
    options.set_keep_alive(Duration::from_secs(15));
    options.set_last_will(LastWill::new(
        format!("{prefix}/status"),
        "offline",
        qos,
        true,
    ));
    if !cfg.username.is_empty() {
        options.set_credentials(&cfg.username, &cfg.password);
    }

    let (client, mut event_loop) = AsyncClient::new(options, 32);

    // Publisher half — periodic snapshot + retained birth message. Owned
    // by the runtime process; dies with it (no explicit join needed).
    let _publisher = {
        let client = client.clone();
        let prefix = prefix.clone();
        let latest = ctx.latest.clone();
        let interval = Duration::from_millis(cfg.publish_interval_ms.max(100) as u64);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // Birth: retained online (delivery rides on the event loop).
            let _ = client
                .publish(format!("{prefix}/status"), qos, true, "online")
                .await;
            loop {
                tick.tick().await;
                let snap = latest.lock().expect("latest mutex").clone();
                let Some(snap) = snap else { continue };
                let payload = snapshot_json(&snap);
                if let Err(e) = client
                    .publish(format!("{prefix}/snapshot"), qos, false, payload)
                    .await
                {
                    tracing::debug!(%e, "northbound publish queue error");
                }
            }
        })
    };

    if cfg.allow_write {
        if let Err(e) = client.subscribe(format!("{prefix}/write"), qos).await {
            tracing::warn!(%e, "northbound write-topic subscribe failed");
        } else {
            tracing::info!(topic = %format!("{prefix}/write"), "northbound write topic enabled");
        }
    }

    tracing::info!(
        broker = %format!("{}:{}", cfg.broker_host, cfg.broker_port),
        prefix = %prefix,
        interval_ms = cfg.publish_interval_ms,
        allow_write = cfg.allow_write,
        "northbound mqtt starting"
    );

    // Event loop half — drives the connection; handles inbound writes.
    loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Packet::Publish(p))) => {
                if cfg.allow_write && p.topic == format!("{prefix}/write") {
                    handle_write(&ctx, &p.payload).await;
                }
            }
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                tracing::info!("northbound mqtt connected");
                // (Re)assert retained online after every (re)connect.
                let _ = client
                    .publish(format!("{prefix}/status"), qos, true, "online")
                    .await;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(%e, "northbound mqtt connection error; retrying");
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }
}

/// `{"name": "...", "value": 12.5}` → one-shot variable write. The
/// variable's IEC type comes from the latest snapshot (REAL values are
/// encoded as IEEE-754 bits for the VM, matching /write semantics).
async fn handle_write(ctx: &NorthboundCtx, payload: &[u8]) {
    #[derive(serde::Deserialize)]
    struct WriteReq {
        name: String,
        value: serde_json::Value,
    }
    let req: WriteReq = match serde_json::from_slice(payload) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(%e, "northbound write: bad payload");
            return;
        }
    };
    let type_name = ctx
        .latest
        .lock()
        .expect("latest mutex")
        .as_ref()
        .and_then(|s| s.vars.iter().find(|v| v.name == req.name))
        .map(|v| v.type_name.clone());
    let Some(type_name) = type_name else {
        tracing::warn!(name = %req.name, "northbound write: unknown variable");
        return;
    };
    let is_real = type_name.eq_ignore_ascii_case("REAL");
    let bits = if is_real {
        let Some(f) = req.value.as_f64() else {
            tracing::warn!(name = %req.name, "northbound write: REAL needs a number");
            return;
        };
        (f as f32).to_bits() as i32
    } else {
        match &req.value {
            serde_json::Value::Bool(b) => *b as i32,
            v => match v.as_i64() {
                Some(i) => i as i32,
                None => {
                    tracing::warn!(name = %req.name, "northbound write: expected int/bool");
                    return;
                }
            },
        }
    };
    match ctx.handle.write_variable(&req.name, bits).await {
        Ok(_) => tracing::info!(name = %req.name, value = %req.value, "northbound write applied"),
        Err(e) => tracing::warn!(name = %req.name, %e, "northbound write failed"),
    }
}

/// VarSnapshot (display strings) → compact JSON with typed values. The
/// IDE-side formatter renders `REAL` as `12.5`, `WORD` as `16#1637`,
/// `BOOL` as `TRUE`/`FALSE`, ints as decimal — decode those so the
/// platform gets numbers, not strings. Anything unparseable passes
/// through as a string rather than being dropped.
fn snapshot_json(snap: &VarSnapshot) -> String {
    let mut values = serde_json::Map::with_capacity(snap.vars.len());
    for v in &snap.vars {
        values.insert(v.name.clone(), decode_display_value(&v.type_name, &v.value));
    }
    serde_json::json!({
        "ts_us": snap.timestamp_us,
        "scan": snap.scan_count,
        "values": values,
    })
    .to_string()
}

fn decode_display_value(type_name: &str, s: &str) -> serde_json::Value {
    let t = type_name.to_ascii_uppercase();
    if t == "BOOL" {
        return serde_json::Value::Bool(s.eq_ignore_ascii_case("true"));
    }
    if let Some(hex) = s.strip_prefix("16#") {
        if let Ok(n) = u32::from_str_radix(hex, 16) {
            return serde_json::json!(n);
        }
    }
    if t == "REAL" || t == "LREAL" {
        if let Ok(f) = s.parse::<f64>() {
            return serde_json::json!(f);
        }
    }
    if let Ok(i) = s.parse::<i64>() {
        return serde_json::json!(i);
    }
    serde_json::Value::String(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_values_decode_to_typed_json() {
        assert_eq!(
            decode_display_value("BOOL", "TRUE"),
            serde_json::json!(true)
        );
        assert_eq!(
            decode_display_value("BOOL", "FALSE"),
            serde_json::json!(false)
        );
        assert_eq!(
            decode_display_value("WORD", "16#1637"),
            serde_json::json!(0x1637)
        );
        assert_eq!(
            decode_display_value("REAL", "12.5"),
            serde_json::json!(12.5)
        );
        assert_eq!(decode_display_value("DINT", "-42"), serde_json::json!(-42));
        assert_eq!(
            decode_display_value("STRING", "hello"),
            serde_json::json!("hello")
        );
    }
}
