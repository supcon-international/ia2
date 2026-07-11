//! Flaky-slave integration test: health flag + reconnect.
//!
//! `run_demo_slave` (and tokio-modbus's TCP server underneath) spawns a
//! task per accepted connection, so killing just the *listener* leaves
//! the master's established connection alive — polls would keep
//! succeeding. To make the link actually die mid-poll, the master talks
//! to the slave through a byte-pipe proxy owned by one task: aborting
//! that task drops its `JoinSet`, which aborts every per-connection
//! pipe, closing the master-side socket → the next poll hits a transport
//! error. Restarting the proxy on the same port lets the adapter's
//! backoff reconnect find the slave again.
//!
//! Scenario:
//!   1. connect through the proxy — healthy, values flow;
//!   2. kill the proxy — consecutive poll failures flip `is_healthy()`
//!      to false (threshold 3);
//!   3. restart the proxy on the same port, change a register on the
//!      slave — the adapter reconnects (100 ms initial backoff), the
//!      flag clears, and the mirror serves the *new* value (proving the
//!      refresh genuinely resumed rather than the flag merely flipping).

use std::net::SocketAddr;
use std::time::Duration;

use iocore::{ChannelValue, IoDevice};
use iomap_modbus::{run_demo_slave, DemoSlave, ModbusDevice};
use project::{
    ModbusChannel, ModbusChannelKind, ModbusConfig, ModbusDataType, ModbusTcpParams,
    ModbusTransport, ModbusWordOrder,
};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

async fn spawn_slave() -> (SocketAddr, DemoSlave) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let slave = DemoSlave::new();
    let s = slave.clone();
    tokio::spawn(async move {
        let _ = run_demo_slave(addr, s).await;
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    (addr, slave)
}

/// Dumb TCP byte pipe `listen_port → upstream`. All per-connection pipes
/// live in a `JoinSet` owned by the accept task, so aborting the returned
/// handle tears down the listener *and* every live connection at once.
async fn spawn_proxy(listen_port: u16, upstream: SocketAddr) -> (u16, JoinHandle<()>) {
    // SO_REUSEADDR so the restart can re-bind the same port immediately
    // even if the previous incarnation left connections in TIME_WAIT.
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_reuseaddr(true).unwrap();
    socket
        .bind(format!("127.0.0.1:{listen_port}").parse().unwrap())
        .unwrap();
    let listener = socket.listen(16).unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let mut pipes = JoinSet::new();
        loop {
            let Ok((mut inbound, _)) = listener.accept().await else {
                break;
            };
            pipes.spawn(async move {
                if let Ok(mut outbound) = TcpStream::connect(upstream).await {
                    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                }
            });
        }
        // Unreachable in practice (the task gets aborted), but dropping
        // `pipes` here is what kills the connections on abort.
        drop(pipes);
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (port, handle)
}

/// Poll `predicate` every 25 ms until it holds or `deadline` elapses.
async fn wait_for(deadline: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let end = tokio::time::Instant::now() + deadline;
    while tokio::time::Instant::now() < end {
        if predicate() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    predicate()
}

#[tokio::test]
async fn link_loss_flips_unhealthy_and_reconnect_recovers() {
    let (slave_addr, slave) = spawn_slave().await;
    slave.input_registers().lock().unwrap()[0] = 41;

    let (port, proxy) = spawn_proxy(0, slave_addr).await;

    let cfg = ModbusConfig {
        transport: ModbusTransport::Tcp(ModbusTcpParams {
            host: "127.0.0.1".into(),
            port,
        }),
        slave_id: 1,
        poll_interval_ms: 50,
        // Short timeout + backoff so the whole outage/recovery cycle
        // fits in test time. Exercises the configurable fields too.
        timeout_ms: Some(250),
        reconnect_backoff_ms: Some(100),
        channels: vec![
            ModbusChannel {
                name: "temp".into(),
                kind: ModbusChannelKind::InputRegister,
                address: 0,
                data_type: ModbusDataType::U16,
                word_order: ModbusWordOrder::HiLo,
            },
            ModbusChannel {
                name: "sp".into(),
                kind: ModbusChannelKind::HoldingRegister,
                address: 5,
                data_type: ModbusDataType::U16,
                word_order: ModbusWordOrder::HiLo,
            },
        ],
    };

    let mut dev = ModbusDevice::connect("flaky".into(), &cfg).await.unwrap();
    assert!(dev.is_healthy(), "fresh connection must start healthy");
    assert_eq!(
        dev.read_channel("temp").await.unwrap(),
        ChannelValue::U16(41),
        "seed poll ran through the proxy"
    );

    // ---- outage: kill the proxy (listener + live connections) ----------
    proxy.abort();
    let _ = proxy.await;

    assert!(
        wait_for(Duration::from_secs(5), || !dev.is_healthy()).await,
        "device must flag unhealthy after 3 consecutive failed polls"
    );

    // Mirror still serves the last-known value while down (no panic, no
    // zeros) — that's the documented stale-mirror behavior.
    assert_eq!(
        dev.read_channel("temp").await.unwrap(),
        ChannelValue::U16(41)
    );

    // Writes while the link is down never park the caller on the dead
    // socket: they queue fire-and-forget (Ok), the poll task fails them
    // and evicts the write-on-change entry, and the scan loop's periodic
    // push retries once the link is back.
    dev.write_channel("sp", ChannelValue::U16(1))
        .await
        .expect("write while down must queue without blocking");

    // ---- recovery: restart the proxy on the same port -------------------
    slave.input_registers().lock().unwrap()[0] = 42;
    let (port2, _proxy2) = spawn_proxy(port, slave_addr).await;
    assert_eq!(port2, port, "proxy must come back on the same port");

    assert!(
        wait_for(Duration::from_secs(10), || dev.is_healthy()).await,
        "device must recover once the slave is reachable again"
    );

    // The refreshed mirror serves the value written DURING the outage —
    // proof the poll loop is live again, not just the flag.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if dev.read_channel("temp").await.unwrap() == ChannelValue::U16(42) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "mirror must refresh to the post-outage value"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // Re-push of the outage-era write (what the scan loop does every scan)
    // must not be dedup-skipped — the failed write was evicted — and must
    // now land on the wire.
    dev.write_channel("sp", ChannelValue::U16(1))
        .await
        .expect("post-recovery write must queue");
    let regs = slave.holding_registers();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if regs.lock().unwrap()[5] == 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "post-recovery write must reach the slave"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    dev.shutdown().await.unwrap();
}
