use crate::framing::Frame;
use crate::protocol::AircraftMessage;

/// Decodes protocol frames into aircraft messages.
///
/// Each implementation is a fully independent decode pipeline. Stateful:
/// maintains known-ICAO set, CPR decode state, and reference position.
pub trait Decoder: Send {
    /// Decode a protocol frame into zero or more aircraft messages.
    ///
    /// Returns an empty Vec for frames that are valid but produce no
    /// message (e.g., Mode-A/C, unknown DF, failed CRC).
    fn decode(&mut self, frame: &Frame) -> Vec<AircraftMessage>;

    /// Set the reference position for local CPR decode.
    fn set_reference_position(&mut self, lat: f64, lon: f64);

    /// Reset decode state (e.g., after reconnection).
    /// Clears CPR state but preserves known-ICAO set.
    fn reset(&mut self);
}
