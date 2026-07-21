# Pluggable Decoder Architecture for adsb-client

## Problem

The BEAST protocol parser in `adsb-client` tangles three concerns into one struct:
transport (TCP byte delivery), framing (BEAST escape handling), and decoding
(Mode-S DF dispatch, ADS-B type codes, BDS disambiguation). This makes it
impossible to swap decode implementations, reuse transports for new protocols,
or add hardware-accelerated backends.

Additionally, the BDS Comm-B decoder uses a sequential first-match-wins
heuristic (`decode_bds()` in `modes.rs`) that tries BDS 6,0 before BDS 5,0.
Ambiguous frames that could be either get consumed by 6,0 - BDS 5,0 data
(roll angle, track angle rate) is effectively invisible.

## Solution

Separate the pipeline into three trait-based layers (Transport, Framer, Decoder)
and provide two independent Decoder implementations: the existing native decoder
and a new rs1090-backed decoder with proper BDS disambiguation.

## Architecture

```
Transport               Framer                  Decoder
(byte delivery)         (message extraction)    (semantic decode)

+-----------+           +---------------+       +----------------+
| TCP       |--Bytes--> | BeastFramer   |--Frame-->| NativeDecoder  |
| NATS*     |           | LineFramer    |       | Rs1090Decoder  |
| Zenoh*    |           | RawModeSFramer*|      | FpgaDecoder*   |
| SDR/USB*  |           +---------------+       +----------------+
| File*     |
+-----------+           * = future
```

### Layer 1: Transport

Delivers raw bytes from a source via an async channel. Wraps the existing
`Connection` in `tcp/mod.rs`.

```rust
pub enum TransportEvent {
    Connected,
    Disconnected,
    Data(Bytes),
    Error(String),
}

#[async_trait]
pub trait Transport: Send {
    async fn recv(&mut self) -> Option<TransportEvent>;
    fn shutdown(&self);
}
```

`TcpTransport` wraps the existing `Connection` struct. Future transports
(NATS, Zenoh, SDR hardware) implement the same trait.

### Layer 2: Framer

Extracts discrete protocol frames from a byte stream. Stateful (maintains an
internal buffer for incomplete frame reassembly).

```rust
pub enum FrameType {
    ModeSShort,   // 7 bytes (DF 0/4/5/11)
    ModeSLong,    // 14 bytes (DF 16/17/18/19/20/21)
    ModeAC,       // 2 bytes
    TextLine,     // SBS-1 CSV line
}

pub struct Frame {
    pub timestamp: Option<u64>,
    pub signal_level: Option<f32>,
    pub data: Bytes,
    pub frame_type: FrameType,
}

pub trait Framer: Send {
    fn feed(&mut self, data: &[u8]);
    fn next_frame(&mut self) -> Option<Frame>;
    fn reset(&mut self);
}
```

`BeastFramer` wraps the existing `FrameDecoder` in `frame.rs`, producing
`Bytes` instead of `Vec<u8>`. `LineFramer` handles SBS-1 newline delimiting
(extracted from `BaseStationParser`).

### Layer 3: Decoder

Decodes protocol frames into aircraft messages. Stateful (maintains known-ICAO
set, CPR decode state, reference position). Each decoder is a fully independent
implementation - no delegation between decoders.

```rust
pub trait Decoder: Send {
    fn decode(&mut self, frame: &Frame) -> Vec<AircraftMessage>;
    fn set_reference_position(&mut self, lat: f64, lon: f64);
    fn reset(&mut self);
}
```

**NativeDecoder**: Wraps all existing decode logic from `modes.rs`, `adsb.rs`,
and `cpr.rs`. Owns `known_icao: HashSet<Icao>` and `cpr_state: HashMap<Icao, CprState>`.
Uses the sequential BDS heuristic (existing behavior preserved).

**Rs1090Decoder**: Uses `rs1090::decode::Message::try_from()` for DF-level
decoding and `rs1090::decode::bds::infer_bds()` for Comm-B BDS disambiguation
with density scoring and cross-field penalties. Owns its own `known_icao` and
`cpr_state`. Maps rs1090 types (`DF`, `DecodedBds`) to our `AircraftMessage`
/ `MessagePayload` types.

Both decoders are complete and standalone. Feature flags select which is compiled.

## ICAO Newtype

Replace `icao: String` throughout with a zero-allocation newtype:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Icao(pub u32);

impl Icao {
    /// Extract ICAO from bytes 1-3 of a Mode-S message (after the DF byte).
    pub fn from_message(data: &[u8]) -> Self {
        Icao(u32::from(data[1]) << 16 | u32::from(data[2]) << 8 | u32::from(data[3]))
    }

    pub fn from_parity(crc: u32) -> Self {
        Icao(crc & 0x00FF_FFFF)
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        u32::from_str_radix(s, 16).ok().map(Icao)
    }
}

impl fmt::Display for Icao {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:06X}", self.0)
    }
}
```

Changes:
- `AircraftMessage.icao`: `String` -> `Icao`
- `known_icao`: `HashSet<String>` -> `HashSet<Icao>`
- `cpr_state`: `HashMap<String, CprState>` -> `HashMap<Icao, CprState>`
- Hex formatting only at display boundaries (UI, logging)

This is a breaking change to the `adsb-client` public API. The `airjedi-bevy`
app will need corresponding updates where it reads `aircraft.icao`.

## Frame Data as Bytes

Replace `Vec<u8>` allocations in the frame pipeline with `bytes::Bytes`:
- `BeastFramer` produces `Frame { data: Bytes, .. }` instead of `BeastFrame { data: Vec<u8> }`
- `TransportEvent::Data(Bytes)` instead of `ConnectionEvent::DataReceived(Vec<u8>)`
- Decoders receive `&Frame` and work on `&[u8]` slices from `frame.data`

`bytes::Bytes` is reference-counted and already in the dep tree via `tokio`.
Cheap to slice and pass across async boundaries without copying.

## New MessagePayload Variants

rs1090 decodes BDS codes we don't currently handle. New variants:

```rust
pub enum MessagePayload {
    // ... existing: Identification, Position, Velocity, Altitude ...

    /// Selected vertical intention (BDS 4,0)
    SelectedAltitude {
        mcp_altitude: Option<i32>,       // MCP/FCU selected altitude (feet)
        fms_altitude: Option<i32>,       // FMS selected altitude (feet)
        barometric_setting: Option<f64>, // QNH (hPa)
    },

    /// Meteorological routine air report (BDS 4,4)
    Meteorological {
        wind_speed: Option<u16>,     // knots
        wind_direction: Option<f64>, // degrees
        temperature: f64,            // Celsius
        pressure: Option<u16>,       // hPa
    },

    /// Meteorological hazard report (BDS 4,5)
    MeteorologicalHazard {
        turbulence: Option<u8>,      // 0-3 severity
        wind_shear: Option<u8>,      // 0-3 severity
        icing: Option<u8>,           // 0-3 severity
        wake_vortex: Option<u8>,     // 0-3 severity
        temperature: Option<f64>,    // Celsius
        pressure: Option<u16>,       // hPa
    },
}
```

The airjedi-bevy app can handle these variants incrementally. Unknown variants
in match arms use `_ => {}` until the UI supports them.

## Feature Flags

```toml
[features]
default = ["decoder-rs1090"]
decoder-rs1090 = ["dep:rs1090"]
# Future:
# decoder-fpga = ["dep:fpga-modes"]
# transport-nats = ["dep:async-nats"]
# transport-zenoh = ["dep:zenoh"]
```

Decoder selection in `Client::spawn()`:

```rust
#[cfg(feature = "decoder-rs1090")]
let decoder: Box<dyn Decoder> = Box::new(Rs1090Decoder::new());

#[cfg(not(feature = "decoder-rs1090"))]
let decoder: Box<dyn Decoder> = Box::new(NativeDecoder::new());
```

The native decoder is always compiled (no feature gate) as the fallback.

## rs1090 Dependency

```toml
[dependencies]
rs1090 = { version = "0.6", optional = true }
bytes = "1"
```

rs1090 is MIT licensed, maintained by Xavier Olive (pyModeS author).
Unconditionally pulls deku, rayon, num-complex, async-stream. The `bds-infer`
feature (default in rs1090) enables density scoring and cross-field penalties.

## Rs1090Decoder Details

### DF Decoding

Uses `rs1090::decode::Message::try_from(bytes)` which returns
`Message { crc: u32, df: DF }`. The `DF` enum contains variants for all
supported downlink formats. The decoder maps these to `MessagePayload`:

| rs1090 DF variant | Our MessagePayload |
|-------------------|-------------------|
| DF::ShortAirAirSurveillance | Altitude |
| DF::SurveillanceAltitudeReply | Altitude |
| DF::SurveillanceIdentityReply | Altitude (with squawk) |
| DF::AllCallReply | Altitude (ICAO registration only) |
| DF::LongAirAirSurveillance | Altitude |
| DF::ADSB(ME::...) | Position, Velocity, Identification |
| DF::CommBAltitudeReply | Altitude + BDS payload |
| DF::CommBIdentityReply | Altitude + BDS payload |

### BDS Disambiguation

For DF20/21 Comm-B replies, calls `rs1090::decode::bds::infer_bds(&mb, Some(icao.0))`
which returns `Vec<DecodedBds>`. Takes the first (highest-confidence) result
and maps:

| DecodedBds variant | Our MessagePayload |
|-------------------|-------------------|
| Bds20(identification) | Identification |
| Bds50(track_and_turn) | Velocity (with roll_angle, track_angle_rate) |
| Bds60(heading_speed) | Velocity (with heading, airspeed) |
| Bds40(selected_vertical) | SelectedAltitude |
| Bds44(meteo_routine) | Meteorological |
| Bds45(meteo_hazard) | MeteorologicalHazard |

### State Management

Rs1090Decoder maintains its own:
- `known_icao: HashSet<Icao>` - populated from DF11/17/18, checked for DF0/4/5/16/20/21
- `cpr_state: HashMap<Icao, CprState>` - odd/even CPR frame pairs for position decoding
- `reference_position: Option<(f64, f64)>` - for local CPR decode

rs1090's `Message::try_from()` computes CRC and uses it as ICAO for non-ADS-B
DFs (the same parity-based extraction we do). The decoder validates this against
`known_icao` before accepting.

CPR position decoding reuses our existing `cpr.rs` logic since rs1090 decodes
individual CPR frames but does not track state across messages.

## Module Layout

```
crates/adsb-client/src/
  lib.rs                    # Client, Icao newtype, public API
  transport/
    mod.rs                  # Transport trait, TransportEvent
    tcp.rs                  # TcpTransport (wraps existing Connection)
  framing/
    mod.rs                  # Framer trait, Frame, FrameType
    beast.rs                # BeastFramer (wraps existing FrameDecoder)
    line.rs                 # LineFramer (SBS-1 line extraction)
  decoder/
    mod.rs                  # Decoder trait
    native/
      mod.rs                # NativeDecoder
      modes.rs              # Existing Mode-S decode (moved from protocol/beast/)
      adsb.rs               # Existing ADS-B decode (moved)
      cpr.rs                # Existing CPR decode (moved)
    rs1090/
      mod.rs                # Rs1090Decoder (behind decoder-rs1090 feature)
      mapping.rs            # DecodedBds -> MessagePayload mapping
  protocol/
    mod.rs                  # AircraftMessage, MessagePayload, Icao, ParseError
  decoder/
    ...
    basestation.rs          # BaseStation CSV decoder (uses LineFramer output)
  tracker/
    mod.rs                  # AircraftTracker (unchanged)
```

## Migration Path

The existing `Protocol` trait is removed. `BeastParser` and `BaseStationParser`
are replaced by the three-layer composition. The `Client` struct wires
Transport + Framer + Decoder together instead of holding a `ParserState` enum.

The `airjedi-bevy` app interacts with `Client` via `get_aircraft()`,
`subscribe()`, etc. - these APIs are unchanged. The only breaking change
visible to the app is `Icao` replacing `String` for ICAO addresses.

## Testing

- Existing tests for `modes.rs`, `adsb.rs`, `cpr.rs`, `frame.rs` remain valid
  and move with their code into the `decoder/native/` and `framing/` modules
- New tests for `Rs1090Decoder` using the same test vectors to verify output
  equivalence with `NativeDecoder` for unambiguous messages
- New tests for BDS disambiguation: craft Comm-B payloads that are ambiguous
  between BDS 5,0 and 6,0, verify Rs1090Decoder picks the correct one
- Integration test: feed a recorded BEAST stream through both decoders, compare
  output message counts and types

## Out of Scope

- New transports (NATS, Zenoh, SDR) - traits are defined but only TCP is implemented
- New framers beyond BEAST and Line
- App-side UI for new MessagePayload variants (SelectedAltitude, Meteorological)
- Runtime decoder switching (compile-time feature flag selection only)
