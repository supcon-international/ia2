//! Regression: an unreachable OPC UA endpoint must NOT wedge startup.
//!
//! `session_retry_limit(-1)` (reconnect forever — the DCS outlives us)
//! made `connect_to_matching_endpoint` / `wait_for_connection` block
//! indefinitely while the endpoint was down. Because the bridge's
//! `connect_devices` awaits each device's `connect()` serially, one
//! unreachable southbound link wedged the entire runtime: the scan loop
//! never started, no program ran, no other device worked. `connect` now
//! bounds the attempt and returns `Err` so the device is skipped (exactly
//! how an unreachable Modbus slave is handled) and the scan loop starts.

use std::time::{Duration, Instant};

use iomap_opcua::OpcuaDevice;
use project::OpcuaConfig;

#[tokio::test]
async fn connect_to_unreachable_endpoint_errors_promptly_instead_of_hanging() {
    // A port nothing listens on → the UA client gets ECONNREFUSED and,
    // with infinite retry, would loop forever without the bounded connect.
    let cfg = OpcuaConfig {
        endpoint_url: "opc.tcp://127.0.0.1:59".to_string(), // discard port, reliably refused
        auth: Default::default(),
        poll_interval_ms: 200,
        channels: vec![],
    };

    let start = Instant::now();
    let result = tokio::time::timeout(
        // Generously above the adapter's internal CONNECT_TIMEOUT (5s):
        // if connect hangs, this outer guard trips and the test fails
        // loudly rather than hanging the suite forever.
        Duration::from_secs(20),
        OpcuaDevice::connect("unreachable".into(), &cfg),
    )
    .await;

    let elapsed = start.elapsed();
    match result {
        Err(_) => {
            panic!("connect() hung past 20s on an unreachable endpoint — the wedge bug is back")
        }
        Ok(Ok(_)) => panic!("connect() unexpectedly succeeded against a dead endpoint"),
        Ok(Err(_)) => {
            // The whole point: it returned an error, bounded. Allow margin
            // over the 5s internal timeout for CI scheduling jitter.
            assert!(
                elapsed < Duration::from_secs(15),
                "connect() took {elapsed:?} — expected to give up near the 5s bound"
            );
        }
    }
}
