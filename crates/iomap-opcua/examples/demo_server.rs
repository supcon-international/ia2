//! Dev-loop OPC UA server: a handful of process-ish tags on
//! `opc.tcp://127.0.0.1:4840/`, writable, with `ft0202_pv` gently
//! wandering so the Monitor shows live movement.
//!
//!     cargo run -p iomap-opcua --example demo_server
//!
//! Pair it with a project device created by `cs device create dcs
//! --protocol opcua` (its default endpoint is exactly this address),
//! then `cs device opcua-browse dcs` / the editor's "Browse server…"
//! panel to pick tags. The demo plays the role of the site's existing
//! DCS/gateway; nothing here is production configuration.

use std::time::Duration;

use opcua::nodes::{ReferenceDirection, VariableBuilder};
use opcua::server::node_manager::memory::{simple_node_manager, SimpleNodeManager};
use opcua::server::ServerBuilder;
use opcua::types::{DataTypeId, NodeId, ObjectId, ReferenceTypeId, Variant};

const NS_URI: &str = "urn:ia2:demo-dcs";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info,opcua=warn")
        .init();

    let (server, handle) = ServerBuilder::new_anonymous("ia2-demo-dcs")
        .application_uri("urn:ia2:demo-dcs-app")
        .product_uri("urn:ia2:demo-dcs-app")
        .host("127.0.0.1")
        .port(4840)
        .with_node_manager(simple_node_manager(
            opcua::server::diagnostics::NamespaceMetadata {
                namespace_uri: NS_URI.to_owned(),
                ..Default::default()
            },
            "simple",
        ))
        .build()
        .expect("server builds");
    let nm = handle
        .node_managers()
        .get_of_type::<SimpleNodeManager>()
        .expect("simple node manager");
    let ns = handle.get_namespace_index(NS_URI).expect("namespace");

    let tags: &[(&str, Variant, DataTypeId)] = &[
        ("ft0202_pv", Variant::Double(12.5), DataTypeId::Double),
        ("ti0301_pv", Variant::Double(64.0), DataTypeId::Double),
        ("flow_sp", Variant::Double(20.0), DataTypeId::Double),
        ("pump_run", Variant::Boolean(false), DataTypeId::Boolean),
        ("step_no", Variant::Int32(0), DataTypeId::Int32),
    ];
    {
        let mut space = nm.address_space().write();
        let objects: NodeId = ObjectId::ObjectsFolder.into();
        let organizes: NodeId = ReferenceTypeId::Organizes.into();
        for (name, value, dt) in tags {
            let id = NodeId::new(ns, *name);
            let node = VariableBuilder::new(&id, *name, *name)
                .data_type(*dt)
                .value(value.clone())
                .writable()
                .build();
            space.insert(
                node,
                Some(&[(&objects, &organizes, ReferenceDirection::Inverse)]),
            );
        }
    }

    // A touch of life: ft0202_pv wanders a slow sine so charts move.
    let wander_nm = nm.clone();
    let wander_id = NodeId::new(ns, "ft0202_pv");
    tokio::spawn(async move {
        let mut t = 0f64;
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            t += 0.5;
            let v = 12.5 + 2.0 * (t / 10.0).sin();
            let mut space = wander_nm.address_space().write();
            if let Some(opcua::nodes::NodeType::Variable(var)) = space.find_mut(&wander_id) {
                // NB: pass a bare Variant — a DataValue here would nest
                // (`Variant::DataValue`) and clients see a wrapped value.
                let _ = var.set_value(&opcua::types::NumericRange::None, Variant::Double(v));
            }
        }
    });

    tracing::info!("demo OPC UA server on opc.tcp://127.0.0.1:4840/ — tags: ft0202_pv ti0301_pv flow_sp pump_run step_no");
    let handle_c = handle.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        handle_c.cancel();
    });
    server.run().await.expect("server runs");
}
