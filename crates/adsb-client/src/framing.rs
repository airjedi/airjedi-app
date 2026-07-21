use bytes::Bytes;

/// The type of a protocol frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    /// Mode-S Short message (7 bytes: DF 0/4/5/11)
    ModeSShort,
    /// Mode-S Long message (14 bytes: DF 16/17/18/19/20/21)
    ModeSLong,
    /// Mode-A/C reply (2 bytes)
    ModeAC,
    /// Text line (SBS-1 CSV)
    TextLine,
}

/// A protocol frame extracted from a byte stream.
#[derive(Debug, Clone)]
pub struct Frame {
    /// MLAT timestamp from the receiver, if available (BEAST only).
    pub timestamp: Option<u64>,
    /// Signal level / RSSI (0.0 - 1.0), if available.
    pub signal_level: Option<f32>,
    /// Raw message bytes (unescaped for BEAST, full line for SBS-1).
    pub data: Bytes,
    /// The frame type.
    pub frame_type: FrameType,
}

/// Extracts discrete protocol frames from a byte stream.
///
/// Implementations handle protocol-specific framing: BEAST escape handling,
/// SBS-1 newline delimiting, raw Mode-S frame boundaries, etc.
pub trait Framer: Send {
    /// Feed raw bytes into the framer's internal buffer.
    fn feed(&mut self, data: &[u8]);

    /// Extract the next complete frame, if available.
    fn next_frame(&mut self) -> Option<Frame>;

    /// Reset internal state (e.g., after reconnection).
    fn reset(&mut self);
}
