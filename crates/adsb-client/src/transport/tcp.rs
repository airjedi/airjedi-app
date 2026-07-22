use bytes::Bytes;

use crate::tcp::{Connection, ConnectionConfig, ConnectionEvent, ConnectionState};
use super::{Transport, TransportEvent};

pub struct TcpTransport {
    connection: Connection,
}

impl TcpTransport {
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            connection: Connection::spawn(config),
        }
    }
}

#[async_trait::async_trait]
impl Transport for TcpTransport {
    async fn recv(&mut self) -> Option<TransportEvent> {
        loop {
            let event = self.connection.recv().await?;
            match event {
                ConnectionEvent::StateChanged(state) => match state {
                    ConnectionState::Connected => return Some(TransportEvent::Connected),
                    ConnectionState::Disconnected => return Some(TransportEvent::Disconnected),
                    ConnectionState::Error(e) => return Some(TransportEvent::Error(e)),
                    ConnectionState::Connecting => continue,
                },
                ConnectionEvent::DataReceived(data) => {
                    return Some(TransportEvent::Data(Bytes::from(data)));
                }
            }
        }
    }

    fn shutdown(&self) {
        self.connection.shutdown();
    }

    fn set_address(&self, address: String) {
        self.connection.set_address(address);
    }

    fn current_address(&self) -> String {
        self.connection.current_address()
    }
}
