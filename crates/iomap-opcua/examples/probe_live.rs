//! One-shot mirror probe against a live endpoint (dev diagnostics):
//! `cargo run -p iomap-opcua --example probe_live`

use iocore::IoDevice;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("debug").init();
    let cfg = project::OpcuaConfig {
        endpoint_url: "opc.tcp://127.0.0.1:4840/".into(),
        auth: Default::default(),
        poll_interval_ms: 200,
        channels: vec![project::OpcuaChannel {
            name: "ft0202_pv".into(),
            node_id: "ns=2;s=ft0202_pv".into(),
            data_type: project::OpcuaDataType::F64,
            access: Default::default(),
            failsafe: None,
        }],
    };
    let mut dev = iomap_opcua::OpcuaDevice::connect("probe".into(), &cfg)
        .await
        .expect("connect");
    for _ in 0..3 {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        println!("ft0202_pv = {:?}", dev.read_channel("ft0202_pv").await);
    }
}
