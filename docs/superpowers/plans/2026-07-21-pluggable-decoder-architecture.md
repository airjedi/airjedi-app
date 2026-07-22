# Pluggable Decoder Architecture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Separate adsb-client into three trait-based layers (Transport, Framer, Decoder), introduce an `Icao` newtype, switch frame data to `bytes::Bytes`, and add an rs1090-backed decoder with proper BDS disambiguation alongside the existing native decoder.

**Architecture:** The pipeline splits into Transport (async byte delivery), Framer (protocol frame extraction), and Decoder (semantic Mode-S decode). Two independent Decoder implementations - NativeDecoder wrapping existing code and Rs1090Decoder using the rs1090 crate - are selected via feature flags. The `Icao` newtype replaces `String` for ICAO addresses throughout.

**Tech Stack:** Rust, Bevy 0.19, rs1090 0.6, bytes crate, tokio async runtime.

## Global Constraints

- Rust edition 2021, Apache-2.0 license
- All public items need `#[must_use]` where appropriate
- Clippy pedantic lints enabled (see existing Cargo.toml `[lints]`)
- Commit style: plain imperative, no prefixes, no emojis
- rs1090 dependency is optional, gated behind `decoder-rs1090` feature
- `bytes` crate version 1.x
- Existing test vectors must continue passing after code moves
- The airjedi-bevy app must compile and run after each task

---

### Task 1: Add `Icao` Newtype and `bytes` Dependency

Introduce the `Icao(u32)` newtype in the protocol module and add `bytes` to
Cargo.toml. This is the foundation that every subsequent task depends on.
This task does NOT yet change any existing code to use `Icao` - that happens
in Task 2.

**Files:**
- Modify: `crates/adsb-client/Cargo.toml`
- Modify: `crates/adsb-client/src/protocol/mod.rs`
- Create: `crates/adsb-client/tests/icao_tests.rs`

**Interfaces:**
- Produces: `Icao` newtype with `from_message(&[u8])`, `from_parity(u32)`, `from_hex(&str)`, `Display` impl. Used by every task from Task 2 onward.

- [ ] **Step 1: Write `Icao` tests**

Create `crates/adsb-client/tests/icao_tests.rs`:

```rust
use adsb_client::Icao;

#[test]
fn from_message_extracts_bytes_1_2_3() {
    // DF byte + ICAO A1B2C3 + padding
    let data = [0x8D, 0xA1, 0xB2, 0xC3, 0x00, 0x00, 0x00];
    let icao = Icao::from_message(&data);
    assert_eq!(icao, Icao(0xA1B2C3));
}

#[test]
fn from_parity_masks_24_bits() {
    let icao = Icao::from_parity(0x00A1B2C3);
    assert_eq!(icao, Icao(0xA1B2C3));

    let icao_overflow = Icao::from_parity(0xFFA1B2C3);
    assert_eq!(icao_overflow, Icao(0xA1B2C3));
}

#[test]
fn from_hex_parses_uppercase_and_lowercase() {
    assert_eq!(Icao::from_hex("A1B2C3"), Some(Icao(0xA1B2C3)));
    assert_eq!(Icao::from_hex("a1b2c3"), Some(Icao(0xA1B2C3)));
    assert_eq!(Icao::from_hex("ZZZZZZ"), None);
}

#[test]
fn display_formats_as_six_digit_uppercase_hex() {
    assert_eq!(format!("{}", Icao(0xA1B2C3)), "A1B2C3");
    assert_eq!(format!("{}", Icao(0x00000F)), "00000F");
}

#[test]
fn icao_is_copy_and_hashable() {
    use std::collections::HashSet;
    let a = Icao(0xA1B2C3);
    let b = a; // Copy
    let mut set = HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --package adsb-client --test icao_tests`
Expected: compilation error - `Icao` not found.

- [ ] **Step 3: Add `bytes` dependency and implement `Icao`**

In `crates/adsb-client/Cargo.toml`, add to `[dependencies]`:
```toml
bytes = "1"
```

In `crates/adsb-client/src/protocol/mod.rs`, add before the `AircraftMessage` struct:

```rust
use std::fmt;

/// ICAO 24-bit aircraft address.
///
/// Stored as a `u32` (only lower 24 bits used) to avoid heap allocation.
/// Format as hex with `Display`: `format!("{icao}")` produces "A1B2C3".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Icao(pub u32);

impl Icao {
    /// Extract ICAO from bytes 1-3 of a Mode-S message (after the DF byte).
    #[must_use]
    pub fn from_message(data: &[u8]) -> Self {
        Self(u32::from(data[1]) << 16 | u32::from(data[2]) << 8 | u32::from(data[3]))
    }

    /// Extract ICAO from the CRC-24 parity remainder.
    #[must_use]
    pub fn from_parity(crc: u32) -> Self {
        Self(crc & 0x00FF_FFFF)
    }

    /// Parse a hex string (e.g., "A1B2C3") into an ICAO address.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        u32::from_str_radix(s, 16).ok().map(Self)
    }
}

impl fmt::Display for Icao {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:06X}", self.0)
    }
}
```

Add `Icao` to the `pub use` in `crates/adsb-client/src/lib.rs`:
```rust
pub use protocol::{AircraftMessage, ..., Icao, ...};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --package adsb-client --test icao_tests`
Expected: all 5 tests pass.

- [ ] **Step 5: Verify full crate still compiles**

Run: `cargo build --package adsb-client`
Expected: success (no existing code changed yet).

- [ ] **Step 6: Commit**

```bash
git add crates/adsb-client/Cargo.toml crates/adsb-client/src/protocol/mod.rs crates/adsb-client/src/lib.rs crates/adsb-client/tests/icao_tests.rs
git commit -m "Add Icao newtype and bytes dependency"
```

---

### Task 2: Migrate Existing Code to `Icao` Newtype

Replace all `String`-based ICAO handling in adsb-client with the `Icao` newtype.
This touches modes.rs, beast/mod.rs, cpr.rs, tracker, and the basestation
parser. The airjedi-bevy app references `aircraft.icao` as `String` in ~76
places - those change to use `Icao` (which implements `Display`).

**Files:**
- Modify: `crates/adsb-client/src/protocol/mod.rs` (AircraftMessage.icao field)
- Modify: `crates/adsb-client/src/protocol/beast/modes.rs` (icao_from_bytes, icao_from_parity, decode_bds, etc.)
- Modify: `crates/adsb-client/src/protocol/beast/mod.rs` (BeastParser known_icao, decode methods)
- Modify: `crates/adsb-client/src/protocol/beast/adsb.rs` (decode_adsb signature)
- Modify: `crates/adsb-client/src/protocol/beast/cpr.rs` (CprState HashMap key)
- Modify: `crates/adsb-client/src/protocol/basestation.rs` (icao field construction)
- Modify: `crates/adsb-client/src/tracker/mod.rs` (Aircraft.icao, AircraftTracker HashMap key)
- Modify: `crates/adsb-client/src/lib.rs` (re-exports)
- Modify: Multiple files in `src/` (airjedi-bevy app - ~76 references to `.icao`)

**Interfaces:**
- Consumes: `Icao` from Task 1
- Produces: `AircraftMessage { icao: Icao, .. }`, `Aircraft { icao: Icao, .. }` used throughout

- [ ] **Step 1: Update `AircraftMessage` to use `Icao`**

In `protocol/mod.rs`, change `AircraftMessage`:
```rust
pub struct AircraftMessage {
    pub icao: Icao,  // was: String
    pub signal_level: Option<f32>,
    pub payload: MessagePayload,
}

impl AircraftMessage {
    #[must_use]
    pub fn icao(&self) -> Icao {  // was: &str
        self.icao
    }
}
```

- [ ] **Step 2: Update `modes.rs` functions**

Change `icao_from_bytes` and `icao_from_parity` to return `Icao`:
```rust
pub fn icao_from_bytes(data: &[u8]) -> Icao {
    Icao::from_message(data)
}

pub fn icao_from_parity(data: &[u8]) -> Icao {
    let crc = crc24(data);
    Icao::from_parity(crc)
}
```

Change `decode_bds` signature:
```rust
pub fn decode_bds(mb: &[u8], icao: Icao, signal_level: Option<f32>) -> Option<AircraftMessage> {
```

Update `try_bds_20`, `try_bds_50`, `try_bds_60` to accept `Icao` and produce `icao` field directly (no `.to_string()`). For BDS 2,0, the adsb_char callsign decoding stays the same - only the `icao` field in the returned `AircraftMessage` changes.

- [ ] **Step 3: Update `beast/mod.rs`**

Change `BeastParser`:
```rust
pub struct BeastParser {
    frame_decoder: FrameDecoder,
    cpr_state: HashMap<Icao, CprState>,  // was: HashMap<String, CprState>
    known_icao: HashSet<Icao>,            // was: HashSet<String>
    reference_position: Option<(f64, f64)>,
}
```

Update `decode_modes_short` and `decode_modes_long` to use `Icao` throughout. All `icao.clone()` calls become simple copies (Icao is Copy). Remove all `.to_string()` calls on ICAO addresses.

- [ ] **Step 4: Update `adsb.rs` and `cpr.rs`**

Change `decode_adsb` signature:
```rust
pub fn decode_adsb(
    tc: u8,
    me: &[u8],
    icao: Icao,                              // was: &str
    cpr_state: &mut HashMap<Icao, CprState>, // was: HashMap<String, CprState>
    reference_position: Option<(f64, f64)>,
) -> Result<Option<MessagePayload>, ParseError> {
```

CPR state lookups change from `cpr_state.entry(icao.to_string())` to `cpr_state.entry(icao)`.

- [ ] **Step 5: Update `basestation.rs`**

Change ICAO construction from `icao: hex_string.to_string()` to `icao: Icao::from_hex(&hex_string).unwrap_or(Icao(0))`.

- [ ] **Step 6: Update `tracker/mod.rs`**

Change `Aircraft.icao` to `Icao`, `AircraftTracker` HashMap key to `Icao`, `process_message` to use `Icao` directly. `get_by_icao` changes signature:
```rust
pub fn get_by_icao(&self, icao: Icao) -> Option<&Aircraft> {
    self.aircraft.get(&icao)
}
```

- [ ] **Step 7: Update airjedi-bevy app references**

The ~76 references to `.icao` in `src/` fall into categories:
- **Display/formatting** (`&aircraft.icao`): works unchanged via `Icao::Display`
- **Equality checks** (`== Some(&aircraft.icao)`): works unchanged via `Icao::PartialEq`
- **Clone calls** (`aircraft.icao.clone()`): replace with `aircraft.icao` (it's Copy)
- **String methods** (`.to_lowercase().contains()`): use `format!("{}", icao).to_lowercase()` or `icao.to_string().to_lowercase()`
- **HashMap/selected_icao**: change `selected_icao: Option<String>` to `Option<Icao>` in the relevant structs

This is mechanical but touches many files. Use `cargo build` after each file to catch misses.

- [ ] **Step 8: Fix all existing tests**

Update test vectors in `modes.rs`, `tracker/mod.rs` etc. to use `Icao` instead of `"A1B2C3".to_string()`. For example:
```rust
// Before:
icao: "A1B2C3".to_string(),
// After:
icao: Icao(0xA1B2C3),
```

- [ ] **Step 9: Run full test suite**

Run: `cargo test --package adsb-client`
Run: `cargo build` (full workspace including airjedi-bevy)
Expected: all tests pass, full app compiles.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "Migrate ICAO handling from String to Icao newtype"
```

---

### Task 3: Define Trait Layer - Transport, Framer, Decoder

Create the three trait definitions and shared types (`Frame`, `FrameType`,
`TransportEvent`). No implementations yet - just the trait contracts.

**Files:**
- Create: `crates/adsb-client/src/transport.rs`
- Create: `crates/adsb-client/src/framing.rs`
- Create: `crates/adsb-client/src/decoder.rs`
- Modify: `crates/adsb-client/src/lib.rs` (add modules)

**Interfaces:**
- Consumes: `Icao`, `AircraftMessage` from Tasks 1-2
- Produces: `Transport` trait, `Framer` trait, `Decoder` trait, `Frame`, `FrameType`, `TransportEvent` - used by all subsequent tasks.

- [ ] **Step 1: Create `transport.rs`**

```rust
use bytes::Bytes;

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
```

- [ ] **Step 2: Create `framing.rs`**

```rust
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
```

- [ ] **Step 3: Create `decoder.rs`**

```rust
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
```

- [ ] **Step 4: Wire modules in `lib.rs`**

Add to `crates/adsb-client/src/lib.rs`:
```rust
pub mod transport;
pub mod framing;
pub mod decoder;
```

And add re-exports:
```rust
pub use framing::{Frame, FrameType, Framer};
pub use transport::{Transport, TransportEvent};
pub use decoder::Decoder;
```

- [ ] **Step 5: Add `async-trait` dependency**

In `crates/adsb-client/Cargo.toml`:
```toml
async-trait = "0.1"
```

- [ ] **Step 6: Verify compilation**

Run: `cargo build --package adsb-client`
Expected: compiles with unused module warnings (no implementations yet).

- [ ] **Step 7: Commit**

```bash
git add crates/adsb-client/src/transport.rs crates/adsb-client/src/framing.rs crates/adsb-client/src/decoder.rs crates/adsb-client/Cargo.toml crates/adsb-client/src/lib.rs
git commit -m "Define Transport, Framer, and Decoder traits"
```

---

### Task 4: Implement BeastFramer and LineFramer

Wrap existing `FrameDecoder` as `BeastFramer` implementing the `Framer` trait.
Extract line-based framing from `BaseStationParser` into `LineFramer`.
Both produce `Frame` with `Bytes` data.

**Files:**
- Create: `crates/adsb-client/src/framing/mod.rs` (move trait defs here)
- Create: `crates/adsb-client/src/framing/beast.rs`
- Create: `crates/adsb-client/src/framing/line.rs`
- Move: `crates/adsb-client/src/protocol/beast/frame.rs` -> `crates/adsb-client/src/framing/beast_frame.rs` (internal, used by BeastFramer)
- Modify: `crates/adsb-client/src/framing.rs` -> becomes `crates/adsb-client/src/framing/mod.rs`

**Interfaces:**
- Consumes: `Framer` trait, `Frame`, `FrameType` from Task 3
- Produces: `BeastFramer` and `LineFramer` structs implementing `Framer`

- [ ] **Step 1: Restructure framing as a module directory**

Move `crates/adsb-client/src/framing.rs` to `crates/adsb-client/src/framing/mod.rs`. Copy `protocol/beast/frame.rs` to `framing/beast_frame.rs` (keep the original for now - the old BeastParser still references it until Task 6 wires everything together).

- [ ] **Step 2: Implement `BeastFramer`**

Create `crates/adsb-client/src/framing/beast.rs`:
```rust
use bytes::Bytes;
use super::{Frame, FrameType, Framer};
use super::beast_frame;

pub struct BeastFramer {
    inner: beast_frame::FrameDecoder,
}

impl BeastFramer {
    pub fn new() -> Self {
        Self {
            inner: beast_frame::FrameDecoder::new(),
        }
    }
}

impl Default for BeastFramer {
    fn default() -> Self {
        Self::new()
    }
}

impl Framer for BeastFramer {
    fn feed(&mut self, data: &[u8]) {
        self.inner.feed(data);
    }

    fn next_frame(&mut self) -> Option<Frame> {
        let beast = self.inner.next_frame()?;
        let frame_type = match beast.msg_type {
            beast_frame::MessageType::ModeAC => FrameType::ModeAC,
            beast_frame::MessageType::ModeSShort => FrameType::ModeSShort,
            beast_frame::MessageType::ModeSLong => FrameType::ModeSLong,
        };
        Some(Frame {
            timestamp: Some(beast.mlat_timestamp),
            signal_level: Some(beast.signal_level as f32 / 255.0),
            data: Bytes::from(beast.data),
            frame_type,
        })
    }

    fn reset(&mut self) {
        self.inner.clear();
    }
}
```

- [ ] **Step 3: Implement `LineFramer`**

Create `crates/adsb-client/src/framing/line.rs`:
```rust
use bytes::Bytes;
use super::{Frame, FrameType, Framer};

pub struct LineFramer {
    buffer: Vec<u8>,
}

impl LineFramer {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(1024),
        }
    }
}

impl Default for LineFramer {
    fn default() -> Self {
        Self::new()
    }
}

impl Framer for LineFramer {
    fn feed(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    fn next_frame(&mut self) -> Option<Frame> {
        let newline_pos = self.buffer.iter().position(|&b| b == b'\n')?;
        let line: Vec<u8> = self.buffer.drain(..=newline_pos).collect();
        let trimmed = line.strip_suffix(&[b'\n'])
            .unwrap_or(&line)
            .strip_suffix(&[b'\r'])
            .unwrap_or(&line);
        if trimmed.is_empty() {
            return None;
        }
        Some(Frame {
            timestamp: None,
            signal_level: None,
            data: Bytes::copy_from_slice(trimmed),
            frame_type: FrameType::TextLine,
        })
    }

    fn reset(&mut self) {
        self.buffer.clear();
    }
}
```

- [ ] **Step 4: Update `framing/mod.rs` to export**

```rust
mod beast_frame;
mod beast;
mod line;

// trait + type defs stay here (from Task 3)...

pub use beast::BeastFramer;
pub use line::LineFramer;
```

- [ ] **Step 5: Test BeastFramer against existing frame tests**

Write a test in `framing/beast.rs` that feeds the same raw bytes as the
existing `frame.rs` tests and verifies `BeastFramer` produces equivalent
`Frame` output with correct `FrameType` and `Bytes` data.

- [ ] **Step 6: Test LineFramer**

Write tests for LineFramer: partial lines, CRLF handling, empty lines,
multiple lines in one feed.

- [ ] **Step 7: Verify compilation and run tests**

Run: `cargo test --package adsb-client`
Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/adsb-client/src/framing/
git commit -m "Implement BeastFramer and LineFramer"
```

---

### Task 5: Implement NativeDecoder

Wrap existing decode logic (modes.rs, adsb.rs, cpr.rs) behind the `Decoder`
trait. The NativeDecoder takes `Frame` input and produces `AircraftMessage`
output using the existing code. This preserves current behavior exactly.

**Files:**
- Create: `crates/adsb-client/src/decoder/mod.rs` (move trait def here)
- Create: `crates/adsb-client/src/decoder/native.rs`
- Modify: `crates/adsb-client/src/decoder.rs` -> becomes `crates/adsb-client/src/decoder/mod.rs`

**Interfaces:**
- Consumes: `Decoder` trait from Task 3, `Frame` from Task 4, existing modes/adsb/cpr code
- Produces: `NativeDecoder` struct implementing `Decoder`

- [ ] **Step 1: Restructure decoder as a module directory**

Move `crates/adsb-client/src/decoder.rs` to `crates/adsb-client/src/decoder/mod.rs`.

- [ ] **Step 2: Implement `NativeDecoder`**

Create `crates/adsb-client/src/decoder/native.rs`. This struct holds the
same state that `BeastParser` currently holds (minus the `FrameDecoder` which
is now in the `BeastFramer`) and calls the same functions in modes.rs and
adsb.rs:

```rust
use std::collections::{HashMap, HashSet};
use crate::framing::{Frame, FrameType};
use crate::protocol::{AircraftMessage, Icao};
use crate::protocol::beast::{modes, adsb, cpr::CprState};
use super::Decoder;

pub struct NativeDecoder {
    cpr_state: HashMap<Icao, CprState>,
    known_icao: HashSet<Icao>,
    reference_position: Option<(f64, f64)>,
}

impl NativeDecoder {
    pub fn new() -> Self {
        Self {
            cpr_state: HashMap::new(),
            known_icao: HashSet::new(),
            reference_position: None,
        }
    }
}

impl Default for NativeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for NativeDecoder {
    fn decode(&mut self, frame: &Frame) -> Vec<AircraftMessage> {
        let data = &frame.data;
        let signal_level = frame.signal_level;

        match frame.frame_type {
            FrameType::ModeAC => vec![],
            FrameType::ModeSShort => {
                self.decode_short(data, signal_level)
                    .into_iter().collect()
            }
            FrameType::ModeSLong => {
                self.decode_long(data, signal_level)
                    .into_iter().collect()
            }
            FrameType::TextLine => vec![], // NativeDecoder doesn't handle SBS-1
        }
    }

    fn set_reference_position(&mut self, lat: f64, lon: f64) {
        self.reference_position = Some((lat, lon));
    }

    fn reset(&mut self) {
        self.cpr_state.clear();
    }
}
```

The `decode_short` and `decode_long` methods contain the same logic as the
current `BeastParser::decode_modes_short` and `decode_modes_long`, calling
into `modes::*` and `adsb::decode_adsb`. Extract these as methods on
`NativeDecoder`.

- [ ] **Step 3: Make modes.rs, adsb.rs, cpr.rs accessible**

These files currently live under `protocol/beast/` with `pub(crate)` or
module-local visibility. They need to be accessible from `decoder/native.rs`.
The cleanest approach: re-export the necessary functions from
`protocol::beast` as `pub(crate)`.

- [ ] **Step 4: Update `decoder/mod.rs`**

```rust
mod native;

// trait def stays here...

pub use native::NativeDecoder;
```

- [ ] **Step 5: Test NativeDecoder with existing test vectors**

Write tests that feed known `Frame` data through `NativeDecoder::decode()`
and verify the same `AircraftMessage` output that the existing `BeastParser`
produces. Use the same test vectors from `modes.rs` tests.

- [ ] **Step 6: Verify and commit**

Run: `cargo test --package adsb-client`
Expected: all tests pass.

```bash
git add crates/adsb-client/src/decoder/
git commit -m "Implement NativeDecoder wrapping existing decode logic"
```

---

### Task 6: Implement TcpTransport and Rewire Client

Wrap existing `Connection` as `TcpTransport` implementing the `Transport`
trait. Rewire `Client` to compose Transport + Framer + Decoder instead of
using the `ParserState` enum and `Protocol` trait.

**Files:**
- Create: `crates/adsb-client/src/transport/mod.rs` (move trait def here)
- Create: `crates/adsb-client/src/transport/tcp.rs`
- Modify: `crates/adsb-client/src/lib.rs` (rewrite Client to use layered architecture)
- Modify: `crates/adsb-client/src/transport.rs` -> becomes `crates/adsb-client/src/transport/mod.rs`

**Interfaces:**
- Consumes: `Transport` trait from Task 3, `BeastFramer`/`LineFramer` from Task 4, `NativeDecoder` from Task 5
- Produces: `TcpTransport` struct, rewired `Client`

- [ ] **Step 1: Restructure transport as a module directory**

Move `crates/adsb-client/src/transport.rs` to `crates/adsb-client/src/transport/mod.rs`.

- [ ] **Step 2: Implement `TcpTransport`**

Create `crates/adsb-client/src/transport/tcp.rs`. This wraps the existing
`Connection` struct from `tcp/mod.rs`:

```rust
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
        let event = self.connection.recv().await?;
        Some(match event {
            ConnectionEvent::StateChanged(ConnectionState::Connected) => {
                TransportEvent::Connected
            }
            ConnectionEvent::StateChanged(ConnectionState::Disconnected) => {
                TransportEvent::Disconnected
            }
            ConnectionEvent::StateChanged(ConnectionState::Error(e)) => {
                TransportEvent::Error(e)
            }
            ConnectionEvent::StateChanged(ConnectionState::Connecting) => {
                return self.recv().await; // skip, wait for next
            }
            ConnectionEvent::DataReceived(data) => {
                TransportEvent::Data(Bytes::from(data))
            }
        })
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
```

- [ ] **Step 3: Rewire `Client` to use layered architecture**

Rewrite `Client` in `lib.rs` to hold `Box<dyn Transport>`, `Box<dyn Framer>`,
and `Box<dyn Decoder>` instead of `ParserState`. The `process_next` and
`process_data` methods delegate through the layers:

```rust
pub struct Client {
    tracker: Arc<RwLock<AircraftTracker>>,
    transport: Box<dyn Transport>,
    framer: Box<dyn Framer>,
    decoder: Box<dyn Decoder>,
    connection_state: Arc<RwLock<ConnectionState>>,
    messages_processed: Arc<AtomicU64>,
}
```

The `process_data` method becomes:
```rust
fn process_data(&mut self, data: &[u8]) {
    self.framer.feed(data);
    while let Some(frame) = self.framer.next_frame() {
        let messages = self.decoder.decode(&frame);
        for msg in messages {
            self.messages_processed.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut tracker) = self.tracker.write() {
                tracker.process_message(msg);
            }
        }
    }
}
```

And `process_next` maps `TransportEvent` variants:
```rust
pub async fn process_next(&mut self) -> bool {
    let event = match self.transport.recv().await {
        Some(event) => event,
        None => return false,
    };
    match event {
        TransportEvent::Connected => {
            self.framer.reset();
            self.decoder.reset();
            // update connection state...
        }
        TransportEvent::Disconnected => { /* update state */ }
        TransportEvent::Data(data) => {
            self.process_data(&data);
        }
        TransportEvent::Error(e) => { /* update state */ }
    }
    true
}
```

`Client::spawn` constructs the appropriate framer and decoder based on
`ProtocolType`:
```rust
let (transport, framer, decoder): (Box<dyn Transport>, Box<dyn Framer>, Box<dyn Decoder>) =
    match config.protocol {
        ProtocolType::BaseStation => {
            conn_config.frame_mode = FrameMode::Line;
            (
                Box::new(TcpTransport::new(conn_config)),
                Box::new(LineFramer::new()),
                Box::new(BaseStationDecoder::new()),
            )
        }
        ProtocolType::Beast => {
            conn_config.frame_mode = FrameMode::Raw;
            let mut dec = NativeDecoder::new(); // or Rs1090Decoder when feature enabled
            if let Some((lat, lon)) = config.tracker.center {
                dec.set_reference_position(lat, lon);
            }
            (
                Box::new(TcpTransport::new(conn_config)),
                Box::new(BeastFramer::new()),
                Box::new(dec),
            )
        }
    };
```

- [ ] **Step 4: Create `BaseStationDecoder`**

A simple decoder that parses SBS-1 CSV lines from `TextLine` frames. Extract
the parse logic from existing `basestation.rs` into a `Decoder` implementation.

- [ ] **Step 5: Remove old `Protocol` trait and `ParserState`**

The `Protocol` trait in `protocol/mod.rs` and the `ParserState` enum in
`lib.rs` are no longer needed. Remove them. The old `BeastParser` struct
can be kept temporarily for reference but should be marked deprecated.

- [ ] **Step 6: Run full test suite and verify app**

Run: `cargo test --package adsb-client`
Run: `cargo build` (full workspace)
Expected: all tests pass, app compiles.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "Rewire Client to use Transport, Framer, Decoder layers"
```

---

### Task 7: Add New MessagePayload Variants

Add `SelectedAltitude`, `Meteorological`, and `MeteorologicalHazard` variants
to `MessagePayload`. Update the tracker to handle them. These are needed
before the Rs1090Decoder can map BDS 4,0/4,4/4,5.

**Files:**
- Modify: `crates/adsb-client/src/protocol/mod.rs`
- Modify: `crates/adsb-client/src/tracker/mod.rs`

**Interfaces:**
- Produces: New `MessagePayload` variants consumed by Rs1090Decoder in Task 8

- [ ] **Step 1: Add variants to `MessagePayload`**

In `protocol/mod.rs`, add after the existing variants:
```rust
/// Selected vertical intention (BDS 4,0).
SelectedAltitude {
    mcp_altitude: Option<i32>,
    fms_altitude: Option<i32>,
    barometric_setting: Option<f64>,
},

/// Meteorological routine air report (BDS 4,4).
Meteorological {
    wind_speed: Option<u16>,
    wind_direction: Option<f64>,
    temperature: f64,
    pressure: Option<u16>,
},

/// Meteorological hazard report (BDS 4,5).
MeteorologicalHazard {
    turbulence: Option<u8>,
    wind_shear: Option<u8>,
    icing: Option<u8>,
    wake_vortex: Option<u8>,
    temperature: Option<f64>,
    pressure: Option<u16>,
},
```

- [ ] **Step 2: Add corresponding fields to `Aircraft` struct**

In `tracker/mod.rs`, add fields:
```rust
pub selected_altitude: Option<i32>,
pub barometric_setting: Option<f64>,
pub wind_speed: Option<u16>,
pub wind_direction: Option<f64>,
pub temperature: Option<f64>,
```

- [ ] **Step 3: Handle new variants in `process_message`**

Add match arms in `AircraftTracker::process_message`:
```rust
MessagePayload::SelectedAltitude { mcp_altitude, barometric_setting, .. } => {
    if let Some(alt) = mcp_altitude {
        aircraft.selected_altitude = Some(alt);
    }
    if let Some(baro) = barometric_setting {
        aircraft.barometric_setting = Some(baro);
    }
}
MessagePayload::Meteorological { wind_speed, wind_direction, temperature, .. } => {
    aircraft.wind_speed = wind_speed;
    aircraft.wind_direction = wind_direction;
    aircraft.temperature = Some(temperature);
}
MessagePayload::MeteorologicalHazard { temperature, .. } => {
    if let Some(t) = temperature {
        aircraft.temperature = Some(t);
    }
}
```

- [ ] **Step 4: Update app-side match arms**

Add `_ => {}` or explicit empty arms for the new variants in any
airjedi-bevy code that matches on `MessagePayload` (if any exists outside
the tracker).

- [ ] **Step 5: Verify and commit**

Run: `cargo test --package adsb-client`
Run: `cargo build`

```bash
git add crates/adsb-client/src/protocol/mod.rs crates/adsb-client/src/tracker/mod.rs
git commit -m "Add SelectedAltitude, Meteorological, and MeteorologicalHazard payload variants"
```

---

### Task 8: Implement Rs1090Decoder

The main event. Implement a full `Decoder` using rs1090 for all DF decoding
and BDS disambiguation. This is a standalone decoder - no delegation to
NativeDecoder.

**Files:**
- Modify: `crates/adsb-client/Cargo.toml` (add rs1090 optional dep)
- Create: `crates/adsb-client/src/decoder/rs1090_decoder.rs`
- Create: `crates/adsb-client/src/decoder/rs1090_mapping.rs`
- Modify: `crates/adsb-client/src/decoder/mod.rs`

**Interfaces:**
- Consumes: `Decoder` trait, `Frame`, `Icao`, `MessagePayload` variants, `CprState` from cpr.rs
- Produces: `Rs1090Decoder` implementing `Decoder`, gated behind `decoder-rs1090` feature

- [ ] **Step 1: Add rs1090 dependency**

In `crates/adsb-client/Cargo.toml`:
```toml
[features]
default = ["decoder-rs1090"]
decoder-rs1090 = ["dep:rs1090"]

[dependencies]
rs1090 = { version = "0.6", optional = true }
```

- [ ] **Step 2: Create the mapping module**

Create `crates/adsb-client/src/decoder/rs1090_mapping.rs`. This contains
pure functions that map rs1090 types to our `MessagePayload`:

```rust
use rs1090::decode::bds::DecodedBds;
use crate::protocol::{Icao, MessagePayload};

pub fn map_bds(decoded: &DecodedBds, icao: Icao) -> Option<MessagePayload> {
    match decoded {
        DecodedBds::Bds20(id) => {
            let callsign: String = /* extract from id struct */;
            Some(MessagePayload::Identification {
                callsign,
                category: None, // BDS 2,0 doesn't carry category
            })
        }
        DecodedBds::Bds50(t) => {
            let speed = t.groundspeed.map(f64::from)
                .or_else(|| t.true_airspeed.map(f64::from))?;
            let track = t.track_angle?;
            Some(MessagePayload::Velocity {
                speed,
                track,
                vertical_rate: None,
                is_on_ground: None,
                heading: None,
                airspeed: t.true_airspeed.map(f64::from),
                roll_angle: t.roll_angle,
                track_angle_rate: t.track_rate,
            })
        }
        DecodedBds::Bds60(h) => {
            let heading = h.magnetic_heading?;
            let speed = h.indicated_airspeed.map(f64::from)?;
            Some(MessagePayload::Velocity {
                speed,
                track: heading,
                vertical_rate: h.barometric_altitude_rate.map(i32::from),
                is_on_ground: None,
                heading: Some(heading),
                airspeed: h.indicated_airspeed.map(f64::from),
                roll_angle: None,
                track_angle_rate: None,
            })
        }
        DecodedBds::Bds40(s) => {
            Some(MessagePayload::SelectedAltitude {
                mcp_altitude: s.selected_altitude_mcp.map(i32::from),
                fms_altitude: s.selected_altitude_fms.map(i32::from),
                barometric_setting: s.barometric_setting,
            })
        }
        DecodedBds::Bds44(m) => {
            Some(MessagePayload::Meteorological {
                wind_speed: m.wind_speed,
                wind_direction: m.wind_direction,
                temperature: m.temperature,
                pressure: m.pressure,
            })
        }
        DecodedBds::Bds45(m) => {
            Some(MessagePayload::MeteorologicalHazard {
                turbulence: m.turbulence,
                wind_shear: m.wind_shear,
                icing: m.icing,
                wake_vortex: m.wake_vortex,
                temperature: m.static_temperature,
                pressure: m.static_pressure,
            })
        }
        _ => None,
    }
}
```

Note: The exact field names on rs1090 structs (e.g., `m.wind_direction`,
`m.turbulence`) must be verified against the rs1090 0.6 API docs at
implementation time. The mapping logic is correct but field names may differ.

- [ ] **Step 3: Implement Rs1090Decoder**

Create `crates/adsb-client/src/decoder/rs1090_decoder.rs`:

```rust
use std::collections::{HashMap, HashSet};
use bytes::Bytes;
use rs1090::decode::Message;
use rs1090::decode::bds::infer_bds;
use crate::framing::{Frame, FrameType};
use crate::protocol::{AircraftMessage, Icao, MessagePayload};
use crate::protocol::beast::cpr::CprState;
use super::Decoder;
use super::rs1090_mapping;

pub struct Rs1090Decoder {
    known_icao: HashSet<Icao>,
    cpr_state: HashMap<Icao, CprState>,
    reference_position: Option<(f64, f64)>,
}

impl Rs1090Decoder {
    pub fn new() -> Self {
        Self {
            known_icao: HashSet::new(),
            cpr_state: HashMap::new(),
            reference_position: None,
        }
    }
}

impl Default for Rs1090Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for Rs1090Decoder {
    fn decode(&mut self, frame: &Frame) -> Vec<AircraftMessage> {
        match frame.frame_type {
            FrameType::ModeSShort | FrameType::ModeSLong => {
                self.decode_modes(&frame.data, frame.signal_level)
            }
            _ => vec![],
        }
    }

    fn set_reference_position(&mut self, lat: f64, lon: f64) {
        self.reference_position = Some((lat, lon));
    }

    fn reset(&mut self) {
        self.cpr_state.clear();
    }
}
```

The `decode_modes` method:
1. Calls `Message::try_from(data)` to parse the raw Mode-S bytes
2. Extracts the ICAO from the parsed message (direct for DF11/17/18, from CRC for others)
3. Validates against `known_icao` for parity-based DFs
4. Maps the rs1090 `DF` variant to our `MessagePayload`
5. For DF20/21, calls `infer_bds()` on the MB field and uses `rs1090_mapping::map_bds()` to convert
6. For DF17/18 ADS-B, maps ME (Message Extended) subtypes to Position/Velocity/Identification
7. For ADS-B position messages, uses our existing `CprState` for position decode

- [ ] **Step 4: Wire into decoder/mod.rs**

```rust
mod native;
#[cfg(feature = "decoder-rs1090")]
mod rs1090_decoder;
#[cfg(feature = "decoder-rs1090")]
mod rs1090_mapping;

pub use native::NativeDecoder;
#[cfg(feature = "decoder-rs1090")]
pub use rs1090_decoder::Rs1090Decoder;
```

- [ ] **Step 5: Update Client to select decoder by feature**

In `Client::spawn` for `ProtocolType::Beast`:
```rust
ProtocolType::Beast => {
    conn_config.frame_mode = FrameMode::Raw;

    #[cfg(feature = "decoder-rs1090")]
    let mut dec = Rs1090Decoder::new();
    #[cfg(not(feature = "decoder-rs1090"))]
    let mut dec = NativeDecoder::new();

    if let Some((lat, lon)) = config.tracker.center {
        dec.set_reference_position(lat, lon);
    }
    (
        Box::new(TcpTransport::new(conn_config)),
        Box::new(BeastFramer::new()),
        Box::new(dec),
    )
}
```

- [ ] **Step 6: Write BDS disambiguation tests**

Test with crafted Comm-B payloads that are ambiguous between BDS 5,0 and 6,0.
Verify that Rs1090Decoder correctly identifies the BDS code via density scoring
while NativeDecoder may pick the wrong one:

```rust
#[test]
fn bds50_not_misidentified_as_bds60() {
    // A payload that looks like valid BDS 6,0 but is actually BDS 5,0
    // (roll angle present, track rate present, typical cruise values)
    let frame = Frame {
        timestamp: None,
        signal_level: Some(0.5),
        data: Bytes::from_static(&[/* DF20 with known BDS 5,0 payload */]),
        frame_type: FrameType::ModeSLong,
    };
    let mut decoder = Rs1090Decoder::new();
    // Pre-populate known_icao so the ICAO passes validation
    decoder.known_icao.insert(Icao(/* test ICAO */));
    let messages = decoder.decode(&frame);
    // Verify we got a Velocity with roll_angle set (BDS 5,0)
    // rather than heading (BDS 6,0)
    assert!(matches!(messages[0].payload,
        MessagePayload::Velocity { roll_angle: Some(_), .. }));
}
```

- [ ] **Step 7: Test output equivalence for unambiguous messages**

Feed the same DF17 ADS-B test vectors through both NativeDecoder and
Rs1090Decoder, verify they produce equivalent `AircraftMessage` output
(same ICAO, same payload fields).

- [ ] **Step 8: Verify full build with and without feature**

Run: `cargo test --package adsb-client` (with decoder-rs1090, default)
Run: `cargo test --package adsb-client --no-default-features` (native only)
Run: `cargo build` (full workspace)
Expected: all pass in both configurations.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "Add Rs1090Decoder with density-based BDS disambiguation"
```

---

### Task 9: Clean Up and Final Integration

Remove deprecated code, update re-exports, ensure the old `Protocol` trait
and `BeastParser` are fully removed. Run the app to verify live data works.

**Files:**
- Modify: `crates/adsb-client/src/lib.rs` (clean re-exports)
- Delete or gut: `crates/adsb-client/src/protocol/beast/mod.rs` (old BeastParser)
- Modify: `crates/adsb-client/src/protocol/mod.rs` (remove Protocol trait)
- Modify: CLAUDE.md if needed

**Interfaces:**
- Consumes: everything from Tasks 1-8
- Produces: clean public API

- [ ] **Step 1: Remove old `Protocol` trait**

Delete the `Protocol` trait, `BeastParser` struct, and `BaseStationParser`
struct. The functionality is now in the three-layer composition.

Keep `protocol/mod.rs` for `AircraftMessage`, `MessagePayload`, `Icao`,
`ParseError`. Keep `protocol/beast/modes.rs`, `adsb.rs`, `cpr.rs` since
NativeDecoder still uses them.

- [ ] **Step 2: Clean up re-exports in `lib.rs`**

Update public API exports:
```rust
pub use protocol::{AircraftMessage, MessagePayload, Icao, ParseError};
pub use transport::{Transport, TransportEvent};
pub use framing::{Frame, FrameType, Framer, BeastFramer, LineFramer};
pub use decoder::{Decoder, NativeDecoder};
#[cfg(feature = "decoder-rs1090")]
pub use decoder::Rs1090Decoder;
pub use tcp::{ConnectionConfig, ConnectionState, FrameMode};
pub use tracker::{Aircraft, AircraftTracker, PositionPoint, TrackerConfig, TrackerEvent};
```

- [ ] **Step 3: Run live data test**

Build and run the full app with a BEAST feed:
```bash
cargo run --release
```
Verify aircraft appear, positions update, BDS 5,0 data (roll angle) is now
visible in the detail panel (if the UI surfaces it).

- [ ] **Step 4: Run full test suite one final time**

```bash
cargo test --workspace
cargo test --package adsb-client --no-default-features
cargo clippy --workspace -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Remove deprecated Protocol trait and finalize layered architecture"
```
