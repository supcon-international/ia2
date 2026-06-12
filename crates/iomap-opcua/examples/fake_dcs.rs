//! Fake DCS — a tiny OPC UA server simulating four plant tags, for
//! developing/demoing IA2's southbound adapter without hardware.
//!
//! Models a slice of a jet-mill line:
//!   ns=2;s=FT0202_PV    Double, RO — feed flow PV, wanders ~12 m³/h
//!   ns=2;s=M0204_AMPS   Double, RO — classifier current, slow sawtooth
//!   ns=2;s=FV0203_CMD   Double, RW — valve command written by IA2
//!   ns=2;s=FEEDER_RUN   Boolean, RW — feeder command written by IA2
//!
//! Run:  cargo run -p iomap-opcua --example fake_dcs
//! Listens on opc.tcp://127.0.0.1:4840/ — client writes are printed to
//! stdout so an end-to-end test can assert "IA2 wrote the DCS tag".

use std::time::Duration;

use opcua::server::address_space::VariableBuilder;
use opcua::server::diagnostics::NamespaceMetadata;
use opcua::server::node_manager::memory::{simple_node_manager, SimpleNodeManager};
use opcua::server::ServerBuilder;
use opcua::types::{
    DataEncoding, DataTypeId, DataValue, DateTime, NodeId, NumericRange, ObjectId,
    TimestampsToReturn, Variant,
};

#[tokio::main]
async fn main() {
    let (server, handle) = ServerBuilder::new_anonymous("IA2 fake DCS")
        .application_uri("urn:ia2:fakedcs")
        .product_uri("urn:ia2:fakedcs")
        .host("127.0.0.1")
        .port(4840)
        .with_node_manager(simple_node_manager(
            NamespaceMetadata {
                namespace_uri: "urn:ia2:fakedcs:tags".to_owned(),
                ..Default::default()
            },
            "fakedcs",
        ))
        .build()
        .expect("server builds");

    let node_manager = handle
        .node_managers()
        .get_of_type::<SimpleNodeManager>()
        .expect("simple node manager registered");
    let ns = handle
        .get_namespace_index("urn:ia2:fakedcs:tags")
        .expect("ns");

    let ft0202 = NodeId::new(ns, "FT0202_PV");
    let m0204 = NodeId::new(ns, "M0204_AMPS");
    let fv0203 = NodeId::new(ns, "FV0203_CMD");
    let feeder = NodeId::new(ns, "FEEDER_RUN");

    {
        let mut space = node_manager.address_space().write();
        let objects: NodeId = ObjectId::ObjectsFolder.into();
        VariableBuilder::new(&ft0202, "FT0202_PV", "FT0202_PV")
            .data_type(DataTypeId::Double)
            .value(12.0f64)
            .organized_by(&objects)
            .insert(&mut *space);
        VariableBuilder::new(&m0204, "M0204_AMPS", "M0204_AMPS")
            .data_type(DataTypeId::Double)
            .value(20.0f64)
            .organized_by(&objects)
            .insert(&mut *space);
        VariableBuilder::new(&fv0203, "FV0203_CMD", "FV0203_CMD")
            .data_type(DataTypeId::Double)
            .value(0.0f64)
            .writable()
            .organized_by(&objects)
            .insert(&mut *space);
        VariableBuilder::new(&feeder, "FEEDER_RUN", "FEEDER_RUN")
            .data_type(DataTypeId::Boolean)
            .value(false)
            .writable()
            .organized_by(&objects)
            .insert(&mut *space);
    }

    println!(
        "fake DCS up at opc.tcp://127.0.0.1:4840  (ns={ns}: FT0202_PV, M0204_AMPS, FV0203_CMD, FEEDER_RUN)"
    );

    // Simulator: PV wanders, classifier current sweeps 20→50 A so a
    // hysteresis interlock trips both directions; print client writes
    // to the RW tags (default write path stores into the address space;
    // we read back and diff each tick).
    {
        let nm = node_manager.clone();
        let subs = handle.subscriptions().clone();
        tokio::spawn(async move {
            let mut t = 0.0f64;
            let mut last_cmd: Option<f64> = None;
            let mut last_run: Option<bool> = None;
            loop {
                let now = DateTime::now();
                let pv = 12.0 + 1.5 * (t / 5.0).sin();
                let amps = 20.0 + (t * 2.0) % 30.0;
                let _ = nm.set_value(&subs, &ft0202, None, DataValue::new_at(pv, now));
                let _ = nm.set_value(&subs, &m0204, None, DataValue::new_at(amps, now));

                let read = |id: &NodeId| -> Option<Variant> {
                    let space = nm.address_space().read();
                    match space.find(id) {
                        Some(opcua::server::address_space::NodeType::Variable(v)) => {
                            v.value(
                                TimestampsToReturn::Neither,
                                &NumericRange::None,
                                &DataEncoding::Binary,
                                0.0,
                            )
                            .value
                        }
                        _ => None,
                    }
                };
                if let Some(Variant::Double(cmd)) = read(&fv0203) {
                    if last_cmd.is_some_and(|p| (p - cmd).abs() > 1e-9) {
                        println!("[fake-dcs] FV0203_CMD written -> {cmd:.3}");
                    }
                    last_cmd = Some(cmd);
                }
                if let Some(Variant::Boolean(run)) = read(&feeder) {
                    if last_run.is_some_and(|p| p != run) {
                        println!("[fake-dcs] FEEDER_RUN written -> {run}");
                    }
                    last_run = Some(run);
                }

                t += 0.5;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
    }

    server.run().await.expect("server runs");
}
