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
use project::{ModbusChannel, ModbusChannelKind, ModbusConfig, ModbusTcpParams, ModbusTransport};
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
        channels: vec![
            ModbusChannel {
                name: "pump".into(),
                kind: ModbusChannelKind::Coil,
                address: 0,
                data_type: Default::default(),
                word_order: Default::default(),
            },
            ModbusChannel {
                name: "valve".into(),
                kind: ModbusChannelKind::Coil,
                address: 1,
                data_type: Default::default(),
                word_order: Default::default(),
            },
            ModbusChannel {
                name: "speed_setpoint".into(),
                kind: ModbusChannelKind::HoldingRegister,
                address: 10,
                data_type: Default::default(),
                word_order: Default::default(),
            },
            ModbusChannel {
                name: "estop_in".into(),
                kind: ModbusChannelKind::DiscreteInput,
                address: 0,
                data_type: Default::default(),
                word_order: Default::default(),
            },
            ModbusChannel {
                name: "temp_in".into(),
                kind: ModbusChannelKind::InputRegister,
                address: 0,
                data_type: Default::default(),
                word_order: Default::default(),
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

    // Sanity: the slave actually saw those writes.
    {
        let coils = slave.coils();
        let guard = coils.lock().unwrap();
        assert!(guard[0], "precondition: coil 0 should be set");
        assert!(guard[1], "precondition: coil 1 should be set");
    }
    {
        let regs = slave.holding_registers();
        let guard = regs.lock().unwrap();
        assert_eq!(guard[10], 1234, "precondition: register 10 should be set");
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
    }
}
