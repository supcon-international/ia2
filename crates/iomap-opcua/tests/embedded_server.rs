//! Full-stack integration against an embedded async-opcua server: the
//! adapter's bulk-read mirror, direct writes, failsafe opt-in contract
//! and the address-space browse all run over a real UA session on
//! loopback — no external server, no mocks at the protocol layer.

use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use iocore::{ChannelValue, IoDevice};
use iomap_opcua::{browse_endpoint, OpcuaDevice};
use opcua::nodes::{ReferenceDirection, VariableBuilder};
use opcua::server::node_manager::memory::{simple_node_manager, SimpleNodeManager};
use opcua::server::{ServerBuilder, ServerHandle};
use opcua::types::{DataTypeId, NodeId, ObjectId, ReferenceTypeId, Variant};
use project::{OpcuaAccess, OpcuaChannel, OpcuaConfig, OpcuaDataType};

/// Sequential ports so parallel tests don't collide. Base chosen out of
/// the way of the IDE server (:3001), demo slave (:5502) and common UA
/// defaults (:4840).
static NEXT_PORT: AtomicU16 = AtomicU16::new(46800);

const NS_URI: &str = "urn:ia2:test-server";

struct TestServer {
    handle: ServerHandle,
    port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    fn endpoint(&self) -> String {
        format!("opc.tcp://127.0.0.1:{}/", self.port)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.cancel();
        self.task.abort();
    }
}

/// Boot a loopback UA server exposing `vars` as writable variables
/// under ObjectsFolder, addressed as `ns=<test>;s=<name>`.
async fn spawn_server(vars: &[(&str, Variant, DataTypeId)]) -> TestServer {
    let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
    let (server, handle) = ServerBuilder::new_anonymous("ia2-test")
        .application_uri("urn:ia2:test-app")
        .product_uri("urn:ia2:test-app")
        .host("127.0.0.1")
        .port(port)
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
        .expect("simple node manager registered");
    let ns = handle
        .get_namespace_index(NS_URI)
        .expect("namespace registered");
    {
        let mut space = nm.address_space().write();
        let objects: NodeId = ObjectId::ObjectsFolder.into();
        let organizes: NodeId = ReferenceTypeId::Organizes.into();
        for (name, value, dt) in vars {
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
    let task = tokio::spawn(async move {
        let _ = server.run().await;
    });
    // The listener needs a beat to come up before clients dial in.
    tokio::time::sleep(Duration::from_millis(300)).await;
    TestServer { handle, port, task }
}

fn tag(name: &str, ns: u16, ty: OpcuaDataType, access: OpcuaAccess) -> OpcuaChannel {
    OpcuaChannel {
        name: name.into(),
        node_id: format!("ns={ns};s={name}"),
        data_type: ty,
        access,
        failsafe: None,
    }
}

fn config(endpoint: String, channels: Vec<OpcuaChannel>) -> OpcuaConfig {
    OpcuaConfig {
        endpoint_url: endpoint,
        auth: Default::default(),
        poll_interval_ms: 60,
        channels,
    }
}

#[tokio::test]
async fn mirror_seeds_then_write_polls_back() {
    let srv = spawn_server(&[
        ("ft0201_pv", Variant::Float(12.5), DataTypeId::Float),
        ("flow_sp", Variant::Float(0.0), DataTypeId::Float),
    ])
    .await;
    let ns = srv.handle.get_namespace_index(NS_URI).unwrap();
    let mut dev = OpcuaDevice::connect(
        "dcs".into(),
        &config(
            srv.endpoint(),
            vec![
                tag("ft0201_pv", ns, OpcuaDataType::F32, OpcuaAccess::Read),
                tag("flow_sp", ns, OpcuaDataType::F32, OpcuaAccess::Write),
            ],
        ),
    )
    .await
    .expect("connect");

    // Seed read: the server-side initial value is in the mirror at once.
    let v = dev.read_channel("ft0201_pv").await.unwrap();
    assert!((v.to_f32() - 12.5).abs() < 1e-6);

    // Write goes out over the session; the poll task reads it back into
    // the mirror from the server (write tags are mirrored too).
    dev.write_channel("flow_sp", ChannelValue::Real(20.0))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let sp = dev.read_channel("flow_sp").await.unwrap();
    assert!((sp.to_f32() - 20.0).abs() < 1e-6, "got {sp:?}");
    assert!(dev.is_healthy());
    dev.shutdown().await.unwrap();
}

#[tokio::test]
async fn writes_to_read_tags_are_rejected() {
    let srv = spawn_server(&[("pv", Variant::Double(1.0), DataTypeId::Double)]).await;
    let ns = srv.handle.get_namespace_index(NS_URI).unwrap();
    let mut dev = OpcuaDevice::connect(
        "dcs".into(),
        &config(
            srv.endpoint(),
            vec![tag("pv", ns, OpcuaDataType::F64, OpcuaAccess::Read)],
        ),
    )
    .await
    .expect("connect");
    let err = dev
        .write_channel("pv", ChannelValue::F64(9.0))
        .await
        .unwrap_err();
    assert!(matches!(err, iocore::IoError::TypeMismatch { .. }));
    dev.shutdown().await.unwrap();
}

#[tokio::test]
async fn failsafe_writes_only_optin_tags() {
    let srv = spawn_server(&[
        ("sp_a", Variant::Float(50.0), DataTypeId::Float),
        ("sp_b", Variant::Float(60.0), DataTypeId::Float),
    ])
    .await;
    let ns = srv.handle.get_namespace_index(NS_URI).unwrap();
    let mut chans = vec![
        tag("sp_a", ns, OpcuaDataType::F32, OpcuaAccess::Write),
        tag("sp_b", ns, OpcuaDataType::F32, OpcuaAccess::Write),
    ];
    chans[0].failsafe = Some(0.0);
    let mut dev = OpcuaDevice::connect("dcs".into(), &config(srv.endpoint(), chans))
        .await
        .expect("connect");

    dev.enter_failsafe().await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    // sp_a (opt-in) went to 0; sp_b held its server-side value — on a
    // supervisory layer untouched tags stay under DCS authority.
    assert!((dev.read_channel("sp_a").await.unwrap().to_f32()).abs() < 1e-6);
    assert!((dev.read_channel("sp_b").await.unwrap().to_f32() - 60.0).abs() < 1e-6);
    dev.shutdown().await.unwrap();
}

#[tokio::test]
async fn browse_lists_variables_with_type_hints() {
    let srv = spawn_server(&[
        ("temp_pv", Variant::Double(21.5), DataTypeId::Double),
        ("run_cmd", Variant::Boolean(false), DataTypeId::Boolean),
    ])
    .await;
    let cfg = config(srv.endpoint(), vec![]);
    let nodes = browse_endpoint(&cfg, None).await.expect("browse");
    let temp = nodes
        .iter()
        .find(|n| n.display_name == "temp_pv")
        .expect("temp_pv listed under ObjectsFolder");
    assert_eq!(temp.node_class, "Variable");
    assert_eq!(temp.data_type.as_deref(), Some("Double"));
    assert_eq!(temp.suggested_channel_type, Some(OpcuaDataType::F64));
    assert!(temp.node_id.contains(";s=temp_pv"));
    let cmd = nodes.iter().find(|n| n.display_name == "run_cmd").unwrap();
    assert_eq!(cmd.suggested_channel_type, Some(OpcuaDataType::Bool));
}
