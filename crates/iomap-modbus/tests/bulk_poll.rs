//! Bulk-poll + 32-bit-type integration: a real TCP round-trip against
//! the in-memory demo slave, exercising span-merged reads, f32 word
//! orders, i16 sign handling, and 32-bit writes on the wire.

use std::net::SocketAddr;
use std::time::Duration;

use iocore::{ChannelValue, IoDevice};
use iomap_modbus::{run_demo_slave, DemoSlave, ModbusDevice};
use project::{
    ModbusChannel, ModbusChannelKind, ModbusConfig, ModbusDataType, ModbusTcpParams,
    ModbusTransport, ModbusWordOrder,
};

async fn spawn_slave() -> (u16, DemoSlave) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let slave = DemoSlave::new();
    let s = slave.clone();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    tokio::spawn(async move {
        let _ = run_demo_slave(addr, s).await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    (port, slave)
}

fn ch(
    name: &str,
    kind: ModbusChannelKind,
    address: u16,
    data_type: ModbusDataType,
    word_order: ModbusWordOrder,
) -> ModbusChannel {
    ModbusChannel {
        name: name.into(),
        kind,
        address,
        data_type,
        word_order,
    }
}

#[tokio::test]
async fn bulk_poll_decodes_sparse_layout_and_32bit_types() {
    let (port, slave) = spawn_slave().await;

    // Seed the slave's input registers BEFORE connecting (connect does a
    // seed poll): u16@0, f32 hi-lo @ 2-3, f32 lo-hi @ 10-11, i16 @ 20.
    let f = 12.7f32.to_bits();
    let (hi, lo) = ((f >> 16) as u16, f as u16);
    {
        let regs = slave.input_registers();
        let mut r = regs.lock().unwrap();
        r[0] = 42;
        r[2] = hi;
        r[3] = lo;
        r[10] = lo; // lo-hi order
        r[11] = hi;
        r[20] = (-40i16) as u16;
    }

    let cfg = ModbusConfig {
        transport: ModbusTransport::Tcp(ModbusTcpParams {
            host: "127.0.0.1".into(),
            port,
        }),
        slave_id: 1,
        poll_interval_ms: 50,
        timeout_ms: None,
        reconnect_backoff_ms: None,
        channels: vec![
            ch(
                "plain",
                ModbusChannelKind::InputRegister,
                0,
                ModbusDataType::U16,
                ModbusWordOrder::HiLo,
            ),
            ch(
                "flow_abcd",
                ModbusChannelKind::InputRegister,
                2,
                ModbusDataType::F32,
                ModbusWordOrder::HiLo,
            ),
            ch(
                "flow_cdab",
                ModbusChannelKind::InputRegister,
                10,
                ModbusDataType::F32,
                ModbusWordOrder::LoHi,
            ),
            ch(
                "temp_i16",
                ModbusChannelKind::InputRegister,
                20,
                ModbusDataType::I16,
                ModbusWordOrder::HiLo,
            ),
            ch(
                "sp_f32",
                ModbusChannelKind::HoldingRegister,
                5,
                ModbusDataType::F32,
                ModbusWordOrder::HiLo,
            ),
        ],
    };

    let mut dev = ModbusDevice::connect("t".into(), &cfg).await.unwrap();

    // Seed poll already ran at connect — values are immediately readable.
    assert_eq!(
        dev.read_channel("plain").await.unwrap(),
        ChannelValue::U16(42)
    );
    assert_eq!(
        dev.read_channel("flow_abcd").await.unwrap(),
        ChannelValue::Real(12.7)
    );
    assert_eq!(
        dev.read_channel("flow_cdab").await.unwrap(),
        ChannelValue::Real(12.7)
    );
    assert_eq!(
        dev.read_channel("temp_i16").await.unwrap(),
        ChannelValue::I32(-40)
    );

    // 32-bit write lands both registers in the configured order. Writes
    // are queued fire-and-forget — wait (bounded) for the poll task to
    // flush the request to the wire.
    dev.write_channel("sp_f32", ChannelValue::Real(3.5))
        .await
        .unwrap();
    let b = 3.5f32.to_bits();
    let regs = slave.holding_registers();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        {
            let r = regs.lock().unwrap();
            if r[5] == (b >> 16) as u16 && r[6] == b as u16 {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                assert_eq!(r[5], (b >> 16) as u16, "high word at base address");
                assert_eq!(r[6], b as u16, "low word at base+1");
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Live update: change a register on the slave; the background poll
    // refreshes the mirror within a couple of intervals.
    {
        let regs = slave.input_registers();
        regs.lock().unwrap()[0] = 99;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        dev.read_channel("plain").await.unwrap(),
        ChannelValue::U16(99)
    );

    dev.shutdown().await.unwrap();
}
