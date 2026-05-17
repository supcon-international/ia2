//! Tiny in-memory Modbus TCP slave for end-to-end demos. 256 coils, 256
//! discrete inputs, 256 holding registers, 256 input registers, all zeroed.
//! Out-of-range addresses return zeros rather than exceptions to keep the
//! demo forgiving.

use std::future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tokio_modbus::prelude::*;
use tokio_modbus::server::tcp::{accept_tcp_connection, Server};
use tokio_modbus::server::Service;

const ADDR_SPACE: usize = 256;

#[derive(Clone, Default)]
pub struct DemoSlave {
    coils: Arc<Mutex<Vec<bool>>>,
    discrete_inputs: Arc<Mutex<Vec<bool>>>,
    holding_registers: Arc<Mutex<Vec<u16>>>,
    input_registers: Arc<Mutex<Vec<u16>>>,
}

impl DemoSlave {
    pub fn new() -> Self {
        Self {
            coils: Arc::new(Mutex::new(vec![false; ADDR_SPACE])),
            discrete_inputs: Arc::new(Mutex::new(vec![false; ADDR_SPACE])),
            holding_registers: Arc::new(Mutex::new(vec![0u16; ADDR_SPACE])),
            input_registers: Arc::new(Mutex::new(vec![0u16; ADDR_SPACE])),
        }
    }

    /// Exposed for ergonomics — handy when seeding discrete-input patterns
    /// or peeking from inside the same process.
    pub fn coils(&self) -> Arc<Mutex<Vec<bool>>> {
        Arc::clone(&self.coils)
    }
    pub fn discrete_inputs(&self) -> Arc<Mutex<Vec<bool>>> {
        Arc::clone(&self.discrete_inputs)
    }
    pub fn holding_registers(&self) -> Arc<Mutex<Vec<u16>>> {
        Arc::clone(&self.holding_registers)
    }
    pub fn input_registers(&self) -> Arc<Mutex<Vec<u16>>> {
        Arc::clone(&self.input_registers)
    }

    fn handle(&self, req: Request<'_>) -> Result<Response, ExceptionCode> {
        match req {
            Request::ReadCoils(addr, count) => Ok(Response::ReadCoils(read_bits(
                &self.coils.lock().unwrap(),
                addr,
                count,
            ))),
            Request::ReadDiscreteInputs(addr, count) => Ok(Response::ReadDiscreteInputs(
                read_bits(&self.discrete_inputs.lock().unwrap(), addr, count),
            )),
            Request::ReadHoldingRegisters(addr, count) => Ok(Response::ReadHoldingRegisters(
                read_words(&self.holding_registers.lock().unwrap(), addr, count),
            )),
            Request::ReadInputRegisters(addr, count) => Ok(Response::ReadInputRegisters(
                read_words(&self.input_registers.lock().unwrap(), addr, count),
            )),
            Request::WriteSingleCoil(addr, value) => {
                if let Some(c) = self.coils.lock().unwrap().get_mut(addr as usize) {
                    *c = value;
                }
                Ok(Response::WriteSingleCoil(addr, value))
            }
            Request::WriteSingleRegister(addr, value) => {
                if let Some(r) = self
                    .holding_registers
                    .lock()
                    .unwrap()
                    .get_mut(addr as usize)
                {
                    *r = value;
                }
                Ok(Response::WriteSingleRegister(addr, value))
            }
            Request::WriteMultipleCoils(addr, values) => {
                let mut coils = self.coils.lock().unwrap();
                for (i, v) in values.iter().enumerate() {
                    if let Some(c) = coils.get_mut(addr as usize + i) {
                        *c = *v;
                    }
                }
                Ok(Response::WriteMultipleCoils(addr, values.len() as u16))
            }
            Request::WriteMultipleRegisters(addr, values) => {
                let mut regs = self.holding_registers.lock().unwrap();
                for (i, v) in values.iter().enumerate() {
                    if let Some(r) = regs.get_mut(addr as usize + i) {
                        *r = *v;
                    }
                }
                Ok(Response::WriteMultipleRegisters(addr, values.len() as u16))
            }
            _ => Err(ExceptionCode::IllegalFunction),
        }
    }
}

fn read_bits(store: &[bool], addr: u16, count: u16) -> Vec<bool> {
    (0..count as usize)
        .map(|i| store.get(addr as usize + i).copied().unwrap_or(false))
        .collect()
}

fn read_words(store: &[u16], addr: u16, count: u16) -> Vec<u16> {
    (0..count as usize)
        .map(|i| store.get(addr as usize + i).copied().unwrap_or(0))
        .collect()
}

impl Service for DemoSlave {
    type Request = Request<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = future::Ready<Result<Self::Response, Self::Exception>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        future::ready(self.handle(req))
    }
}

/// Binds a Modbus TCP slave on `addr` and serves forever. Cancelable by
/// dropping the future.
pub async fn run_demo_slave(addr: SocketAddr, slave: DemoSlave) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "demo modbus slave listening");
    let server = Server::new(listener);
    let on_connected = move |stream, socket_addr| {
        // Each accepted connection gets its own clone of the slave handle
        // (the underlying state is Arc'd so all clones share storage).
        let slave = slave.clone();
        async move {
            let new_service = move |_addr| Ok::<_, std::io::Error>(Some(slave.clone()));
            accept_tcp_connection(stream, socket_addr, new_service)
        }
    };
    server
        .serve(&on_connected, |e| {
            tracing::warn!(%e, "modbus connection error");
        })
        .await
}
