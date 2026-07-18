//! SocketCAN transport — Linux only (the kernel interface doesn't exist
//! elsewhere). Dev machines use `interface = "_sim"`; a real edge box
//! names its interface (`can0`) after ops brought it up with
//! `ip link set can0 up type can txqueuelen 1000 bitrate 500000`.

use async_trait::async_trait;
use iocore::IoError;
use socketcan::tokio::CanSocket;
use socketcan::{CanFrame as ScFrame, EmbeddedFrame, Frame, StandardId};

use crate::bus::CanBus;
use crate::frame::CanFrame;

pub struct SocketcanBus {
    socket: CanSocket,
}

impl SocketcanBus {
    pub fn open(interface: &str) -> Result<Self, IoError> {
        let socket = CanSocket::open(interface).map_err(|e| {
            IoError::Connect(format!(
                "socketcan open '{interface}': {e} (is the interface up? \
                 `ip link set {interface} up type can bitrate …`)"
            ))
        })?;
        Ok(Self { socket })
    }
}

#[async_trait]
impl CanBus for SocketcanBus {
    async fn send(&mut self, frame: CanFrame) -> Result<(), IoError> {
        let id = StandardId::new(frame.id)
            .ok_or_else(|| IoError::Transport(format!("bad CAN id {:#x}", frame.id)))?;
        let f = ScFrame::new(id, frame.payload())
            .ok_or_else(|| IoError::Transport("payload exceeds 8 bytes".into()))?;
        self.socket
            .write_frame(f)
            .await
            .map_err(|e| IoError::Transport(format!("socketcan write: {e}")))
    }

    async fn recv(&mut self) -> Result<CanFrame, IoError> {
        loop {
            let f = self
                .socket
                .read_frame()
                .await
                .map_err(|e| IoError::Transport(format!("socketcan read: {e}")))?;
            // CANopen's predefined connection set is all 11-bit ids;
            // skip extended/remote/error frames.
            if f.is_extended() || f.is_remote_frame() || f.is_error_frame() {
                continue;
            }
            return Ok(CanFrame::new(f.raw_id() as u16, f.data()));
        }
    }
}
