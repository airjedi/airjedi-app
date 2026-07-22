mod tcp;

use bytes::Bytes;

pub use tcp::TcpTransport;

/// Events from a transport source.
#[derive(Debug, Clone)]
pub enum TransportEvent {
    /// Connection established.
    Connected,
    /// Connection lost (will attempt reconnect).
    Disconnected,
    /// Raw bytes received from the source.
    Data(Bytes),
    /// Transport-level error.
    Error(String),
}

/// Async byte delivery from any source (TCP, NATS, Zenoh, SDR, file).
///
/// Implementations manage connection lifecycle and produce raw bytes.
/// The transport knows nothing about the protocol carried over it.
#[async_trait::async_trait]
pub trait Transport: Send {
    /// Wait for the next event from the transport.
    /// Returns `None` when the transport is shut down.
    async fn recv(&mut self) -> Option<TransportEvent>;

    /// Initiate a graceful shutdown.
    fn shutdown(&self);

    /// Change the endpoint address (for transports that support it).
    fn set_address(&self, _address: String) {}

    /// Get the current endpoint address.
    fn current_address(&self) -> String {
        String::new()
    }
}
