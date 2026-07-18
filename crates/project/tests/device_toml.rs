//! Device configs must round-trip through TOML byte-faithfully enough
//! to re-load — the store persists them as `devices/<name>.toml`. The
//! risky shapes are the tagged enums (ProtocolConfig's `protocol` tag,
//! CanopenTransport's `kind` tag, ModbusTransport's untagged compat),
//! which TOML represents as tables; a serde attribute change that
//! breaks this surfaces here, not in a customer project.

use project::{
    CanopenAccess, CanopenChannel, CanopenConfig, CanopenDataType, CanopenTransport, Device,
    OpcuaAccess, OpcuaAuth, OpcuaChannel, OpcuaConfig, OpcuaDataType, ProtocolConfig,
};

fn roundtrip(device: &Device) -> Device {
    let text = toml::to_string_pretty(device).expect("serialises to TOML");
    toml::from_str(&text).expect("parses back")
}

#[test]
fn canopen_device_roundtrips_through_toml() {
    let device = Device {
        name: "servo".into(),
        config: ProtocolConfig::Canopen(CanopenConfig {
            interface: "can0".into(),
            node_id: 34,
            bitrate: Some(500_000),
            poll_interval_ms: 100,
            heartbeat_timeout_ms: 3000,
            start_on_connect: true,
            channels: vec![
                CanopenChannel {
                    name: "statusword".into(),
                    index: 0x6041,
                    sub_index: 0,
                    data_type: CanopenDataType::U16,
                    access: CanopenAccess::Read,
                    transport: CanopenTransport::Tpdo {
                        slot: 1,
                        byte_offset: 0,
                    },
                    failsafe: None,
                },
                CanopenChannel {
                    name: "controlword".into(),
                    index: 0x6040,
                    sub_index: 0,
                    data_type: CanopenDataType::U16,
                    access: CanopenAccess::Write,
                    transport: CanopenTransport::Rpdo {
                        slot: 1,
                        byte_offset: 0,
                    },
                    failsafe: None,
                },
                CanopenChannel {
                    name: "target_velocity".into(),
                    index: 0x60FF,
                    sub_index: 0,
                    data_type: CanopenDataType::I32,
                    access: CanopenAccess::Write,
                    transport: CanopenTransport::Sdo,
                    failsafe: Some(0.0),
                },
            ],
        }),
    };
    let back = roundtrip(&device);
    let ProtocolConfig::Canopen(cfg) = &back.config else {
        panic!("protocol tag lost");
    };
    assert_eq!(cfg.node_id, 34);
    assert_eq!(cfg.bitrate, Some(500_000));
    assert_eq!(cfg.channels.len(), 3);
    assert_eq!(
        cfg.channels[0].transport,
        CanopenTransport::Tpdo {
            slot: 1,
            byte_offset: 0
        }
    );
    assert_eq!(cfg.channels[2].transport, CanopenTransport::Sdo);
    assert_eq!(cfg.channels[2].failsafe, Some(0.0));
}

#[test]
fn canopen_minimal_toml_gets_defaults() {
    // A hand-authored file with only the essentials — serde defaults
    // fill the rest, matching how agents write configs incrementally.
    let text = r#"
name = "io_block"
protocol = "canopen"
interface = "_sim"
node_id = 5

[[channels]]
name = "di_0"
index = 24576
data_type = "bool"
"#;
    let device: Device = toml::from_str(text).expect("parses");
    let ProtocolConfig::Canopen(cfg) = &device.config else {
        panic!("wrong protocol");
    };
    assert_eq!(cfg.poll_interval_ms, 100);
    assert_eq!(cfg.heartbeat_timeout_ms, 3000);
    assert!(cfg.start_on_connect);
    assert_eq!(cfg.channels[0].access, CanopenAccess::Read);
    assert_eq!(cfg.channels[0].transport, CanopenTransport::Sdo);
    assert_eq!(cfg.channels[0].sub_index, 0);
}

#[test]
fn opcua_device_roundtrips_through_toml() {
    let device = Device {
        name: "dcs".into(),
        config: ProtocolConfig::Opcua(OpcuaConfig {
            endpoint_url: "opc.tcp://10.0.0.10:4840".into(),
            auth: OpcuaAuth::UserPassword {
                username: "op".into(),
                password: "secret".into(),
            },
            poll_interval_ms: 500,
            channels: vec![OpcuaChannel {
                name: "ft0202_pv".into(),
                node_id: "ns=2;s=FT0202.PV".into(),
                data_type: OpcuaDataType::F64,
                access: OpcuaAccess::Read,
                failsafe: None,
            }],
        }),
    };
    let back = roundtrip(&device);
    let ProtocolConfig::Opcua(cfg) = &back.config else {
        panic!("protocol tag lost");
    };
    assert!(matches!(cfg.auth, OpcuaAuth::UserPassword { .. }));
    assert_eq!(cfg.channels[0].node_id, "ns=2;s=FT0202.PV");
    assert_eq!(cfg.channels[0].data_type, OpcuaDataType::F64);
}
