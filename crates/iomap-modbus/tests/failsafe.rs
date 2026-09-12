//! End-to-end test: connect a real `ModbusDevice` master to the in-process
//! `DemoSlave`, write non-zero values, trip `enter_failsafe`, and confirm
//! every writable channel is zero on the wire afterwards.
//!
//! Why not a unit test inside `client.rs`? `enter_failsafe` actually sends
//! Modbus PDUs through `tokio-modbus`. The cheapest fixture for that is
//! `DemoSlave` bound to an ephemeral localhost port — same path the IDE's
//! demo mode uses, so the test exercises the production code path.

use std::time::Duration;

use iocore::{ChannelValue, IoDevice};
use iomap_modbus::{run_demo_slave, DemoSlave, ModbusDevice};
use project::{
    ModbusAccess, ModbusChannel, ModbusChannelKind, ModbusConfig, ModbusTcpParams, ModbusTransport,
};
use tokio::net::TcpListener;

/// Bind the demo slave to `127.0.0.1:0` (kernel-assigned port), spawn the
/// server, and return the assigned port. Caller is expected to drop the
/// returned `SlaveHandle` to tear down; we leak the spawned task on
/// purpose since test processes exit immediately after.
async fn spawn_slave() -> (u16, DemoSlave) {
    // Pre-bind to discover the port, then drop the listener so the server
    // can re-bind. There's a brief race between the drop and the server's
    // bind; in practice it's never lost because tokio reuses SO_REUSEADDR
    // semantics on Linux + macOS for this test pattern.
    let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let slave = DemoSlave::new();
    let slave_clone = slave.clone();
    tokio::spawn(async move {
        let addr = format!("127.0.0.1:{port}").parse().unwrap();
        let _ = run_demo_slave(addr, slave_clone).await;
    });

    // Wait for the server to come up. 200 ms is generous and the test
    // suite tolerates the extra delay since it only runs once.
    tokio::time::sleep(Duration::from_millis(200)).await;
    (port, slave)
}

fn config_with_mixed_channels(port: u16) -> ModbusConfig {
    ModbusConfig {
        transport: ModbusTransport::Tcp(ModbusTcpParams {
            host: "127.0.0.1".into(),
            port,
        }),
        slave_id: 1,
        poll_interval_ms: 100,
        timeout_ms: None,
        reconnect_backoff_ms: None,
        channels: vec![
            ModbusChannel {
                name: "pump".into(),
                kind: ModbusChannelKind::Coil,
                address: 0,
                data_type: Default::default(),
                word_order: Default::default(),
                access: Default::default(),
            },
            ModbusChannel {
                name: "valve".into(),
                kind: ModbusChannelKind::Coil,
                address: 1,
                data_type: Default::default(),
                word_order: Default::default(),
                access: Default::default(),
            },
            ModbusChannel {
                name: "speed_setpoint".into(),
                kind: ModbusChannelKind::HoldingRegister,
                address: 10,
                data_type: Default::default(),
                word_order: Default::default(),
                access: Default::default(),
            },
            ModbusChannel {
                name: "estop_in".into(),
                kind: ModbusChannelKind::DiscreteInput,
                address: 0,
                data_type: Default::default(),
                word_order: Default::default(),
                access: Default::default(),
            },
            ModbusChannel {
                name: "temp_in".into(),
                kind: ModbusChannelKind::InputRegister,
                address: 0,
                data_type: Default::default(),
                word_order: Default::default(),
                access: Default::default(),
            },
            // The NX6 shape: a MEASUREMENT the coupler maps into a
            // holding register. Writable kind, read-only truth — the
            // access field is what keeps failsafe (and writes) off it.
            ModbusChannel {
                name: "level_meas".into(),
                kind: ModbusChannelKind::HoldingRegister,
                address: 20,
                data_type: Default::default(),
                word_order: Default::default(),
                access: ModbusAccess::Read,
            },
        ],
    }
}

#[tokio::test]
async fn enter_failsafe_zeroes_coils_and_holding_registers_on_the_wire() {
    let (port, slave) = spawn_slave().await;
    let cfg = config_with_mixed_channels(port);
    let mut dev = ModbusDevice::connect("test".into(), &cfg).await.unwrap();

    // Seed: drive every writable channel to a non-zero value.
    dev.write_channel("pump", ChannelValue::Bool(true))
        .await
        .unwrap();
    dev.write_channel("valve", ChannelValue::Bool(true))
        .await
        .unwrap();
    dev.write_channel("speed_setpoint", ChannelValue::U16(1234))
        .await
        .unwrap();

    // Sanity: the slave actually saw those writes. They are queued
    // fire-and-forget, so wait (bounded) for the poll task to flush them.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let landed = {
            let coils = slave.coils();
            let regs = slave.holding_registers();
            let c = coils.lock().unwrap();
            let r = regs.lock().unwrap();
            c[0] && c[1] && r[10] == 1234
        };
        if landed {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "precondition: seed writes must reach the slave"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // Preload the read-only measurement register on the slave side, as
    // if the coupler were publishing a live value there.
    {
        let regs = slave.holding_registers();
        regs.lock().unwrap()[20] = 4321;
    }

    // Trip failsafe — this is the path the scan loop will take when the
    // watchdog fires or on graceful shutdown.
    dev.enter_failsafe().await.unwrap();

    // Verify the wire side: zero coils + zero holding registers.
    {
        let coils = slave.coils();
        let guard = coils.lock().unwrap();
        assert!(!guard[0], "coil 0 must be cleared by failsafe");
        assert!(!guard[1], "coil 1 must be cleared by failsafe");
    }
    {
        let regs = slave.holding_registers();
        let guard = regs.lock().unwrap();
        assert_eq!(guard[10], 0, "register 10 must be zeroed by failsafe");
        assert_eq!(
            guard[20], 4321,
            "access=read holding register must NOT be written by failsafe \
             (no write command may reach it at all)"
        );
    }
}

/// Write permission is enforced on the NORMAL write path too, not just
/// failsafe: a read-marked holding register rejects writes up front.
#[tokio::test]
async fn write_to_access_read_channel_is_rejected() {
    let (port, slave) = spawn_slave().await;
    let cfg = config_with_mixed_channels(port);
    let mut dev = ModbusDevice::connect("test".into(), &cfg).await.unwrap();

    {
        let regs = slave.holding_registers();
        regs.lock().unwrap()[20] = 777;
    }
    // Queued write path is fire-and-forget; the rejection surfaces as
    // the value never reaching the wire.
    let _ = dev.write_channel("level_meas", ChannelValue::U16(1)).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let regs = slave.holding_registers();
    assert_eq!(
        regs.lock().unwrap()[20],
        777,
        "write to an access=read channel must never reach the wire"
    );
}
