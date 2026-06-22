---
date: 2026-06-20
updated: 2026-06-20
status: design
tags: [airjedi, fusion, tracking, architecture, rerun, nats, kalman-filter, multi-domain]
---

# Multi-Sensor Fusion Pipeline and Tracking System

**Date:** 2026-06-20
**Status:** Design (Rev 3 - performance/scaling fixes for 100s of sensors and targets)
**Approach:** Separate crate, Bevy-native (Approach B)

## Overview

A general-purpose multi-sensor, multi-domain fusion engine for AirJedi, implemented as a standalone Bevy-aware crate (`airjedi-fusion`). The system ingests observations from heterogeneous sensor sources, correlates them to tracks, fuses them through configurable estimation filters, and publishes fused track state both locally (for rendering) and across tiers (via NATS/JetStream).

Designed for a hierarchical fusion architecture where edge nodes (drones, robots), regional ground stations, and global data centers each run their own fusion instance. Each tier consumes raw sensor data and produces fused estimates that become inputs to other tiers. A fused track from an upstream tier is itself a sensor source with its own confidence and uncertainty.

**Multi-domain by design:** The system tracks targets across air (fixed-wing, rotary-wing, drones, balloons, missiles), ground (vehicles, people), maritime (surface vessels, submarines), and space (satellites, debris). Target domain and classification drive filter selection, association gating, lifecycle timeouts, and display symbology.

### Key Design Influences

- **Rerun.io patterns:** Immutable time-series storage with dual-timeline indexing, late-binding fusion (raw observations never modified), explicit coordinate frame transforms, archetype-based partial data handling for sensor dropouts.
- **ECS advantage:** Bevy's Entity-Component-System maps naturally to the problem - entities are tracks, components are sensor data/fused state/quality/classification, systems are the pipeline stages. Adding a new sensor type or target domain means adding a new ingest system and component, not refactoring core logic.
- **Standard tracking theory:** Internal filter state maintained in ECEF Cartesian coordinates (not geodetic) per Bar-Shalom [1]. Out-of-sequence measurement handling for DIL resilience [5]. Interacting Multiple Model (IMM) for mixed target dynamics [3].

### Hierarchical Tier Architecture

```
Edge tier (drone/robot swarm)     -> local tactical fusion -> fused tracks
Regional tier (ground stations)   -> regional fusion       -> refined tracks
Global tier (data centers)        -> strategic fusion       -> correlated tracks
```

Each tier both consumes raw sensor data AND produces fused estimates that become inputs to the next tier up. Higher tier fused output can flow back down as "upstream intelligence" to be fused with local raw data.

## Crate Structure

```
airjedi-bevy/
+-- Cargo.toml              (workspace root)
+-- src/                    (airjedi-bevy app)
+-- adsb-client/            (existing crate)
+-- airjedi-fusion/
    +-- Cargo.toml
    +-- src/
        +-- lib.rs
        +-- plugin.rs       FusionPlugin (Bevy plugin entry point)
        +-- sensor.rs       SensorSource trait, SensorObservation, SensorMeta
        +-- datastore.rs    TimelineStore (Arrow-backed append-only log)
        +-- track.rs        TrackEntity components, classification, lifecycle
        +-- filter/
        |   +-- mod.rs      TrackFilter trait, FilterVariant enum
        |   +-- ekf.rs      6-DOF Extended Kalman Filter (ECEF)
        |   +-- ekf4.rs     4-DOF surface target filter
        |   +-- imm.rs      Interacting Multiple Model wrapper
        +-- associator/
        |   +-- mod.rs      Associator trait
        |   +-- gnn.rs      Global Nearest Neighbor
        |   +-- spatial_index.rs  Grid-based spatial pre-filter for gating
        +-- transport/
        |   +-- mod.rs      TransportSink/TransportSource traits
        |   +-- nats.rs     NATS JetStream implementation
        +-- coord.rs        Coordinate frame definitions, ECEF/ENU/geodetic conversions
        +-- oosm.rs         Out-of-sequence measurement handling
        +-- classification.rs  Target domain, category, affiliation
```

### Dependencies

- `bevy` (0.18, minimal features - no rendering)
- `nalgebra` - filter math (generic-dimensional matrices)
- `arrow` / `arrow-array` / `arrow-buffer` - columnar datastore
- `parquet` - cold storage persistence
- `async-nats` - NATS JetStream transport
- `prost` / `prost-types` - protobuf wire format
- `serde` - configuration serialization
- `lapjv` or equivalent - Jonker-Volgenant assignment for GNN associator
- `chrono` - timestamp handling

### API Boundary with AirJedi

The app interacts with the fusion crate through:

1. Adding `FusionPlugin` to the Bevy app
2. Implementing sensor adapters that produce `SensorObservation`s
3. Querying `TrackerState`, `TrackQuality`, and `TargetClassification` components to drive rendering
4. Configuring the pipeline via `FusionConfig` resource

The fusion crate knows nothing about tiles, cameras, UI, or rendering.

## Core Type Definitions

### Identity Types

```rust
/// Unique track identifier (UUID, generated internally)
pub struct TrackId(pub Uuid);

/// Cooperative target identifier from a sensor
pub struct TargetId {
    pub domain: TargetDomain,
    pub id: String,
    pub id_type: IdentifierType,
}

pub enum TargetDomain {
    Air,
    Ground,
    Maritime,
    Space,
    Subsurface,
    Person,
}

pub enum IdentifierType {
    // Aviation
    Icao,           // 24-bit ICAO hex address (ADS-B)
    Callsign,       // flight number (e.g., "UAL1234")
    ModeA,          // Mode A squawk code
    RemoteId,       // FAA Remote ID (drones)
    TailNumber,     // aircraft registration

    // Maritime
    Mmsi,           // Maritime Mobile Service Identity (AIS)
    ImoNumber,      // International Maritime Organization number

    // Space
    NoradId,        // NORAD catalog number
    CosparId,       // COSPAR international designator

    // Ground
    LicensePlate,
    Vin,

    // Universal
    Uuid,
    Rfid,
    Custom,
}
```

### Target Classification

Drives filter selection, association gating, lifecycle timeouts, and display symbology.

```rust
#[derive(Component)]
pub struct TargetClassification {
    pub domain: TargetDomain,
    pub category: TargetCategory,
    pub specific_type: Option<String>,  // "B737-800", "DJI Mavic 3", "Arleigh Burke DDG"
    pub affiliation: Affiliation,
    pub confidence: f32,                // classification confidence 0.0-1.0
}

pub enum TargetCategory {
    // Air
    FixedWing, RotaryWing, Drone, Balloon, Missile, Rocket,
    // Space
    Satellite, SpaceDebris, LaunchVehicle,
    // Ground
    GroundVehicle, Person, AnimalOrWildlife,
    // Maritime
    SurfaceVessel, Submarine,
    // Unknown
    Unknown,
}

pub enum Affiliation {
    Friendly,
    Hostile,
    Neutral,
    Unknown,
}
```

Classification can be set by cooperative sensors (ADS-B includes aircraft type), inferred from kinematics (speed/altitude/maneuver pattern), or assigned by an operator. Confidence degrades over time if not refreshed.

### State Representation

**Critical design decision:** Filter state is maintained internally in ECEF (Earth-Centered, Earth-Fixed) Cartesian coordinates, NOT geodetic lat/lon [1][2]. This avoids:
- Mixing radians and meters in the covariance matrix (numerical conditioning)
- Longitude scale varying with latitude (1 degree of lon = 111km at equator, 40km at 70N)
- Nonlinear spherical corrections in the process model

Geodetic coordinates are used only at sensor ingestion (converting incoming lat/lon to ECEF) and at the display boundary (converting ECEF back to lat/lon for rendering).

```rust
/// ECEF Cartesian state vector (meters, meters/second)
/// 6-DOF: [x, y, z, vx, vy, vz]
/// 4-DOF surface: [x, y, vx, vy] (constrained to WGS-84 ellipsoid)
/// 9-DOF maneuvering: [x, y, z, vx, vy, vz, ax, ay, az]
pub type StateVector = DVector<f64>;
pub type StateCovariance = DMatrix<f64>;

pub enum StateVectorType {
    Cartesian6Dof,     // air, ground 3D
    Surface4Dof,       // ships, ground vehicles (2D + velocity)
    Maneuvering9Dof,   // aircraft in turn, accelerating targets
    // Future: Orbital6Dof (Keplerian elements for space objects)
}

/// Observation-level uncertainty (sensor-reported or derived from sensor specs)
pub struct ObservationCovariance {
    pub matrix: DMatrix<f64>,   // measurement noise covariance R
}

/// Auxiliary information attached to an observation
pub struct ObservationMetadata {
    pub signal_strength: Option<f32>,   // dBm or SNR
    pub accuracy_category: Option<u8>,  // e.g., NACp for ADS-B
    pub source_label: String,           // human-readable source name
}
```

### Coordinate Frames and Conversions

All coordinate conversions are handled in `coord.rs`. Sensors report in their native frame; the fusion pipeline converts to ECEF internally. Display systems convert back to geodetic for rendering.

```rust
/// Defines the coordinate frame a sensor reports in
pub enum CoordinateFrame {
    /// WGS-84 geodetic (lat/lon/alt) - ADS-B, GPS, AIS, upstream fused
    Wgs84,
    /// Earth-Centered Earth-Fixed Cartesian - internal filter frame
    Ecef,
    /// Local East-North-Up relative to a reference point
    Enu { origin_lat: f64, origin_lon: f64, origin_alt: f64 },
    /// Spherical relative to sensor position (range/azimuth/elevation) - radar
    SensorSpherical { sensor_lat: f64, sensor_lon: f64, sensor_alt: f64 },
    /// Bevy world coordinates (used after conversion for rendering)
    BevyWorld,
}

/// Conversion functions (in coord.rs)
/// geodetic_to_ecef(lat_rad, lon_rad, alt_m) -> (x, y, z)
/// ecef_to_geodetic(x, y, z) -> (lat_rad, lon_rad, alt_m)
/// ecef_to_enu(x, y, z, ref_lat, ref_lon, ref_alt) -> (e, n, u)
/// spherical_to_ecef(range, az, el, sensor_ecef) -> (x, y, z)
```

### Pipeline Configuration

```rust
#[derive(Resource)]
pub struct FusionConfig {
    pub store: StoreConfig,
    pub lifecycle: LifecycleProfiles,     // per-target-category lifecycle params
    pub associator: AssociatorConfig,
    pub filter_defaults: FilterSelectionConfig,
    pub oosm: OosmConfig,
    pub transport: Option<NatsTransportConfig>,  // None = standalone mode
    pub node_id: String,
    pub tier: FusionTier,
}

/// Per-target-category lifecycle and filter parameters
pub struct LifecycleProfiles {
    pub profiles: HashMap<TargetCategory, LifecycleProfile>,
    pub default: LifecycleProfile,
}

pub struct LifecycleProfile {
    pub lifecycle: TrackLifecycleConfig,
    pub filter_type: FilterSelection,
    pub gate_params: GateParams,
}
```

## Sensor Abstraction Layer

Every data source - raw hardware sensor, upstream fused track, or simulated feed - is a `SensorSource` that produces `SensorObservation`s.

### Sensor Identity

```rust
pub struct SensorId {
    pub id: String,
    pub kind: SensorKind,
    pub tier: FusionTier,
    pub coordinate_frame: CoordinateFrame,
}

pub enum SensorKind {
    // Aviation
    AdsbReceiver,
    MlatNetwork,
    PrimaryRadar,
    SecondaryRadar,
    // Maritime
    AisReceiver,
    MaritimeRadar,
    Sonar,
    // Multi-domain
    OpticalTracker,
    RfTracker,
    GpsTracker,
    // Space
    SpaceSurveillanceRadar,
    // Meta
    UpstreamFusedTrack,
    Simulated,
}

pub enum FusionTier {
    Edge,
    Regional,
    Global,
}
```

### Sensor Observation

The universal data unit produced by every source:

```rust
pub struct SensorObservation {
    pub sensor_id: SensorId,
    pub timestamp: Timestamp,           // when it happened (nanosecond precision)
    pub receipt_time: Timestamp,        // when we received it
    pub target_id: Option<TargetId>,    // cooperative ID if known
    pub measurement: Measurement,
    pub covariance: ObservationCovariance,
    pub classification_hint: Option<TargetCategory>,  // sensor's best guess at target type
    pub metadata: ObservationMetadata,
}
```

### Measurement Types

Different sensors observe different quantities. The filter's measurement model is selected based on which variant arrives.

```rust
pub enum Measurement {
    /// Full 3D position + velocity (ADS-B, GPS tracker, upstream fused 3D)
    PositionVelocity3D {
        lat: f64, lon: f64, alt_m: Option<f64>,
        vel_north: Option<f64>, vel_east: Option<f64>, vel_down: Option<f64>,
        heading: Option<f64>,
    },
    /// 2D surface position + velocity (AIS, ground GPS, license plate reader)
    PositionVelocity2D {
        lat: f64, lon: f64,
        speed_over_ground: Option<f64>,
        course_over_ground: Option<f64>,
    },
    /// Range/azimuth/elevation from a known point (primary radar, sonar)
    Spherical {
        range_m: f64, azimuth_rad: f64, elevation_rad: Option<f64>,
        range_rate: Option<f64>,  // Doppler, if available
    },
    /// Bearing-only (passive optical/RF, passive sonar)
    BearingOnly {
        azimuth_rad: f64, elevation_rad: Option<f64>,
    },
    /// Depth measurement (active sonar, pressure sensor)
    DepthBearing {
        depth_m: f64,
        azimuth_rad: Option<f64>,
        range_m: Option<f64>,
    },
    /// Pre-fused state estimate from upstream tier
    FusedEstimate {
        state_type: StateVectorType,
        state: StateVector,
        covariance: StateCovariance,
        track_quality: f32,
    },
}
```

### SensorSource Trait

```rust
pub trait SensorSource: Send + Sync + 'static {
    fn sensor_id(&self) -> &SensorId;
    fn poll_observations(&mut self) -> Vec<SensorObservation>;
}
```

ADS-B accuracy categories (NACp, SIL) map to covariance values in the adapter. AIS position accuracy maps similarly. Upstream fused tracks carry their own covariance from the producing tier.

## Embedded Datastore (Arrow-backed Timeline Store)

An in-process, append-only, time-indexed store for raw observations and fused state. Serves two purposes: real-time fusion queries and persistent logging for offline replay.

**Key architectural decisions:**

1. **No observation entities.** The datastore is the sole repository for raw observations. Observations are NOT spawned as individual ECS entities - this avoids entity accumulation (200 sensors x 1 Hz = 60,000 entities in 5 minutes, plus radar sweep data would push into hundreds of thousands). Instead, observations are inserted into the store with an `associated_track: Option<TrackId>` field. The associator updates this field rather than reparenting entities.

2. **VecDeque hot buffer, Arrow for cold storage only.** Arrow's columnar arrays (TimestampArray, StructArray) are designed for batch analytics - immutable once built, requiring full-array reconstruction to append or evict. This is wrong for a hot ring buffer with constant appends and evictions at thousands of operations per second. The hot buffer uses plain `VecDeque<StoredObservation>` (O(1) append/evict, cache-friendly array-of-structs layout). Arrow is used only for cold storage serialization - when observations age out, they're batch-converted to Arrow RecordBatches and written to Parquet. This matches Rerun's actual architecture: plain Rust data structures for the hot path, Arrow as the serialization format [8].

3. **No spatial queries in the store.** Spatial filtering is the associator's concern (via its spatial index). The store's queries are keyed by track ID and time range, which are served efficiently by the HashMap + VecDeque structure.

### Design Principles

- Immutable writes - observations never modified after insertion (association metadata is a separate index)
- Dual timeline indexing - `sensor_time` and `receipt_time`
- Association is a store-level operation, not an ECS entity operation
- Hot path uses plain Rust data structures; Arrow/Parquet for persistence only

### Store Structure

```rust
pub struct TimelineStore {
    /// Recent observations, partitioned by track (or unassociated)
    hot_buffer: HotBuffer,
    /// Background writer for persistent storage
    cold_writer: Option<ColdStorageWriter>,
    config: StoreConfig,
}

pub struct HotBuffer {
    /// Observations already associated to a track, keyed by TrackId
    by_track: HashMap<TrackId, VecDeque<StoredObservation>>,
    /// Observations not yet associated (pending association)
    unassociated: Vec<StoredObservation>,
}

pub struct StoredObservation {
    pub observation: SensorObservation,
    pub associated_track: Option<TrackId>,
    pub store_time: Timestamp,
}

pub struct StoreConfig {
    pub hot_retention: Duration,             // e.g., 60s
    pub max_observations_per_track: usize,   // ring buffer cap per track
    pub cold_enabled: bool,
    pub cold_path: PathBuf,
    pub cold_rotation: Duration,             // file rotation interval, e.g., 5 min
}
```

### Query Interface

```rust
impl TimelineStore {
    /// Insert a new raw observation (called by drain system only)
    pub fn insert(&mut self, obs: SensorObservation);

    /// Associate an observation with a track (called by associator)
    pub fn associate(&mut self, obs_idx: usize, track_id: &TrackId);

    /// All observations for a track within a time window
    pub fn query_range(&self, track_id: &TrackId, from: Timestamp, to: Timestamp)
        -> Vec<&StoredObservation>;

    /// Latest observation per sensor for a track (for fusion update step)
    pub fn latest_per_sensor(&self, track_id: &TrackId)
        -> HashMap<SensorId, &StoredObservation>;

    /// All unassociated observations (for the associator)
    pub fn unassociated(&self) -> &[StoredObservation];
}
```

Note: no `query_spatial()` - spatial filtering is handled by the associator's spatial index, not the store.

### Persistence

A background system (`flush_cold_storage`) runs on Bevy's `AsyncComputeTaskPool`. When observations age out of the hot VecDeque buffers, they are batch-converted to Arrow RecordBatches and written to Parquet files partitioned by time window. This decouples the hot path (plain Rust, fast) from the persistence format (Arrow/Parquet, optimized for analytics and replay).

### Relationship to NATS

The store is local only. NATS handles distribution. Observations arriving via NATS enter the local store through `insert()` like any local sensor. Fused tracks published to NATS are read from track components, not from the store.

## Track Lifecycle and Entity Model

### Entity Structure

Track entities carry their tracker state (filter + fused estimate combined), quality metrics, and classification. Raw observations live in the `TimelineStore`, not as child entities.

```
TrackEntity
  +-- Track              (identity, cooperative IDs)
  +-- TrackerState        (filter internals + fused state estimate)
  +-- TrackQuality        (status, confidence, staleness)
  +-- TargetClassification (domain, category, affiliation)
```

### Track Components

```rust
#[derive(Component)]
pub struct Track {
    pub id: TrackId,
    pub cooperative_ids: Vec<TargetId>,   // may have multiple (ICAO + callsign + ModeA)
    pub created_at: Timestamp,
    pub last_update: Timestamp,
}

/// Combined filter state and fused estimate.
/// The filter IS the fused state - they are the same thing.
/// External consumers read state() and covariance() without
/// knowing which filter variant is running internally.
#[derive(Component)]
pub struct TrackerState {
    variant: FilterVariant,
    state_type: StateVectorType,
}

impl TrackerState {
    pub fn state(&self) -> &StateVector { ... }
    pub fn covariance(&self) -> &StateCovariance { ... }
    pub fn state_type(&self) -> StateVectorType { ... }
    pub fn predict(&mut self, dt: f64) { ... }
    pub fn update(&mut self, obs: &SensorObservation) -> FilterResult { ... }
    pub fn innovation(&self, obs: &SensorObservation) -> Innovation { ... }

    /// Geodetic position for display (converted from internal ECEF)
    pub fn position_geodetic(&self) -> (f64, f64, Option<f64>) { ... }
    /// Velocity in NED frame for display
    pub fn velocity_ned(&self) -> (f64, f64, f64) { ... }
}

#[derive(Component)]
pub struct TrackQuality {
    pub status: TrackStatus,
    pub sensor_count: u8,
    pub update_rate: f32,
    pub staleness: Duration,
    pub confidence: f32,
}

pub enum TrackStatus {
    Tentative,
    Confirmed,
    Coasting,
    Lost,
}
```

### Track Lifecycle State Machine

```
New unassociated observation (no matching track)
  -> Spawn TrackEntity (Tentative)
    -> N confirmations within time window -> Confirmed
      -> Continuous updates -> stays Confirmed
      -> No updates for coast_timeout -> Coasting (dead-reckoning, uncertainty grows)
        -> Sensor reacquires -> Confirmed
        -> No updates for lost_timeout -> Lost -> Despawn
```

### Lifecycle Configuration (Per-Target-Category)

Different target types have different lifecycle needs. A parked car should coast for hours; a missile for seconds. Lifecycle parameters are keyed by `TargetCategory`:

```rust
pub struct TrackLifecycleConfig {
    pub confirm_threshold: u32,         // observations needed to confirm
    pub confirm_window: Duration,       // must arrive within this window
    pub coast_timeout: Duration,        // Confirmed -> Coasting
    pub lost_timeout: Duration,         // Coasting -> Lost
    pub cleanup_delay: Duration,        // grace period before despawn
}

// Example profiles:
// FixedWing:      confirm=3/10s,  coast=15s,  lost=60s
// Drone:          confirm=3/5s,   coast=10s,  lost=30s
// Missile:        confirm=2/3s,   coast=5s,   lost=15s
// GroundVehicle:  confirm=3/30s,  coast=300s, lost=3600s
// SurfaceVessel:  confirm=3/60s,  coast=600s, lost=7200s
// Satellite:      confirm=2/120s, coast=3600s, lost=86400s
// Person:         confirm=3/10s,  coast=30s,  lost=120s
```

### Lifecycle Systems

- `track_initiation_system` - spawns TrackEntity from unassociated observations in the store
- `track_status_system` - transitions status based on staleness and per-category config
- `track_cleanup_system` - despawns Lost tracks, archives final state
- `track_merge_system` - merges convergent tentative tracks
- `track_classification_system` - updates TargetClassification from sensor hints and kinematics

Cooperative ID resolution: when an ADS-B observation with ICAO gets associated with an anonymous radar-initiated track, the track's `cooperative_ids` list is updated and retained through subsequent sensor dropouts.

## Filter System

### Design Decisions

**ECEF internal state [1][2]:** All filters maintain state in Earth-Centered Earth-Fixed Cartesian coordinates. This makes the constant-velocity process model truly linear (no Jacobians for propagation), avoids numerical conditioning issues from mixing radians and meters, and eliminates the longitude-scale-varies-with-latitude problem. Sensor observations are converted from their native frame (geodetic, spherical, ENU) to ECEF at the measurement model boundary.

**Combined filter + state component [6]:** The `TrackerState` component wraps the filter variant and exposes the fused estimate. This prevents the filter internals and fused state from getting out of sync (they are the same data structure).

**IMM for mixed dynamics [3]:** When a target transitions between motion regimes (cruise to maneuver, hover to transit), a single motion model produces large residuals. The Interacting Multiple Model (IMM) filter runs multiple models in parallel and blends their outputs by model probability. The `FilterVariant` enum includes an IMM wrapper.

### TrackFilter Trait

```rust
pub trait TrackFilter: Send + Sync + 'static {
    fn predict(&mut self, dt: f64);
    fn update(&mut self, observation: &SensorObservation) -> FilterResult;
    fn state(&self) -> &StateVector;
    fn covariance(&self) -> &StateCovariance;
    fn innovation(&self, observation: &SensorObservation) -> Innovation;
    fn initialize(&mut self, observation: &SensorObservation);

    /// State history for OOSM rollback (last N predict/update steps)
    fn state_history(&self) -> &StateHistory;
    fn rollback_to(&mut self, timestamp: Timestamp) -> bool;
}

pub struct Innovation {
    pub residual: DVector<f64>,
    pub covariance: DMatrix<f64>,
    pub mahalanobis_distance: f64,
}

pub enum FilterResult {
    Updated,
    OutlierRejected { distance: f64 },
    DivergenceDetected,
}
```

### Filter Variants

```rust
pub enum FilterVariant {
    /// 6-DOF constant velocity in ECEF - air targets, 3D ground
    Ekf6Dof(Ekf6Dof),
    /// 4-DOF surface-constrained - ships, ground vehicles
    Ekf4Dof(Ekf4Dof),
    /// Coordinated turn model - maneuvering aircraft
    CoordinatedTurn(CoordinatedTurnEkf),
    /// Interacting Multiple Model - blends multiple motion models
    /// by probability. Handles transitions between cruise/maneuver/hover.
    Imm(ImmFilter),

    // Future variants:
    // OrbitalUkf - Keplerian elements for satellites (SGP4/SDP4 propagator)
    // BallisticEkf - ballistic trajectory for missiles/projectiles
    // StationaryFilter - near-zero-velocity targets (parked, hovering)
}

/// Filter selection based on target category
pub enum FilterSelection {
    /// Use a specific filter variant
    Fixed(FilterVariantType),
    /// Use IMM with these model combinations
    Imm(Vec<FilterVariantType>),
    /// Auto-select based on observed kinematics
    Adaptive,
}

// Default selections:
// FixedWing    -> Imm([Ekf6Dof, CoordinatedTurn])
// RotaryWing   -> Imm([Ekf6Dof, CoordinatedTurn])
// Drone        -> Imm([Ekf6Dof, CoordinatedTurn])
// Balloon      -> Ekf6Dof (low process noise)
// Missile      -> Ekf6Dof (high process noise) or future BallisticEkf
// GroundVehicle -> Ekf4Dof
// SurfaceVessel -> Ekf4Dof
// Submarine    -> Ekf6Dof (with depth)
// Satellite    -> Ekf6Dof (future: OrbitalUkf)
// Person       -> Ekf4Dof (very low process noise)
// Unknown      -> Imm([Ekf6Dof, Ekf4Dof])
```

### 6-DOF EKF in ECEF (Initial Implementation)

State vector: `[x, y, z, vx, vy, vz]` in ECEF meters and m/s.

```rust
pub struct Ekf6Dof {
    pub x: SVector<f64, 6>,
    pub p: SMatrix<f64, 6, 6>,
    pub q_config: ProcessNoiseConfig,
    pub history: StateHistory,
}

pub struct ProcessNoiseConfig {
    pub position_noise: f64,    // m^2/s^3
    pub velocity_noise: f64,    // m^2/s^5
}
```

### Process Model

Constant-velocity in ECEF. For time step `dt`:
- `x_new = x + vx * dt` (and similarly for y, z)
- Velocity unchanged: `vx_new = vx`
- Process noise `Q` is a block diagonal scaled by `dt`

Because state is in Cartesian ECEF, the process model is purely linear - no Jacobians needed for the prediction step [1]. This is a significant advantage over geodetic-frame filters.

### Measurement Models

One per `Measurement` variant. The measurement model converts from the observation's native frame to ECEF (or vice versa) and computes the Jacobian H:

- **PositionVelocity3D** - Convert geodetic (lat/lon/alt) to ECEF. H maps ECEF state to ECEF observation. Velocity converted from NED to ECEF via rotation matrix.
- **PositionVelocity2D** - Surface-constrained. Convert lat/lon to ECEF on WGS-84 ellipsoid (alt=0 or terrain height). Only horizontal state elements observed.
- **Spherical** - Nonlinear transform from ECEF state to (range, azimuth, elevation) relative to sensor ECEF position. Jacobian computed analytically. Range rate (Doppler) provides direct velocity information when available.
- **BearingOnly** - Highly nonlinear, constrains direction only. Covariance stays large along unobserved range dimension until triangulated from multiple observations or sensors.
- **DepthBearing** - Sonar measurement. Depth constrains the Z component; bearing constrains direction.
- **FusedEstimate** - Upstream state estimate. If in the same ECEF frame, direct observation. Uses covariance intersection [4] when cross-correlations between local and upstream filters are unknown (which they always are in a decentralized architecture).

### Outlier Rejection

Before update, Mahalanobis distance checked against chi-squared gate. Gate threshold varies by target category and measurement dimensionality (chi-squared degrees of freedom = measurement dimension). Rejected observations stay in the datastore for forensics but don't influence fused state.

### Out-of-Sequence Measurement (OOSM) Handling

In DIL scenarios with NATS JetStream replay, observations frequently arrive with timestamps older than the filter's current state. Naively discarding them wastes information; naively applying them corrupts the filter.

**Approach: state history with rollback-and-replay [5].**

```rust
pub struct StateHistory {
    /// Ring buffer of (timestamp, state, covariance) snapshots
    snapshots: VecDeque<StateSnapshot>,
    max_depth: usize,  // e.g., 10 steps
}

pub struct StateSnapshot {
    pub timestamp: Timestamp,
    pub state: StateVector,
    pub covariance: StateCovariance,
}

pub struct OosmConfig {
    pub max_lag: Duration,     // max age of late observation to accept (e.g., 30s)
    pub history_depth: usize,  // number of state snapshots to retain (e.g., 10)
}
```

When a late observation arrives:
1. Find the most recent state snapshot before the observation's timestamp
2. Roll back the filter to that snapshot
3. Insert the late observation at the correct chronological position
4. Replay all subsequent observations forward to the current time
5. If no snapshot is old enough, or the observation exceeds `max_lag`, discard it

### Fusion Update System

```rust
fn fusion_update_system(
    time: Res<Time>,
    store: Res<TimelineStore>,
    oosm_config: Res<OosmConfig>,
    mut tracks: Query<(&Track, &mut TrackerState, &mut TrackQuality)>,
) {
    let dt = time.delta_secs_f64();
    for (track, mut tracker, mut quality) in &mut tracks {
        // 1. Predict forward
        tracker.predict(dt);

        // 2. Gather new observations from the store
        let obs = store.query_range(&track.id, track.last_update, now());

        // 3. Sort by timestamp, handle OOSM
        for ob in obs.iter().sorted_by_key(|o| o.timestamp) {
            if ob.timestamp < tracker.state_history().latest_timestamp() {
                // Out-of-sequence: rollback and replay
                handle_oosm(&mut tracker, ob, &store, &oosm_config);
            } else {
                // In-sequence: normal update
                match tracker.update(ob) {
                    FilterResult::Updated => { /* update quality */ },
                    FilterResult::OutlierRejected { .. } => { /* log */ },
                    FilterResult::DivergenceDetected => tracker.initialize(ob),
                }
            }
        }
    }
}
```

## Associator System

### Associator Trait

```rust
pub trait Associator: Send + Sync + 'static {
    fn associate(
        &self,
        observations: &[&StoredObservation],
        tracks: &[(&Track, &TrackerState, &TargetClassification)],
        config: &AssociatorConfig,
    ) -> AssociationResult;
}

pub struct AssociationResult {
    pub assignments: Vec<Assignment>,
    pub unassigned_observations: Vec<usize>,
    pub unassigned_tracks: Vec<usize>,
}

pub struct Assignment {
    pub observation_idx: usize,
    pub track_idx: usize,
    pub distance: f64,
    pub confidence: f32,
}
```

### Target-Class-Aware Gating

A single gate threshold fails for mixed target environments. A gate calibrated for commercial aircraft (200+ knots) will either miss a walking person or create false associations for a missile. Gate parameters vary by target category:

```rust
pub struct AssociatorConfig {
    pub gate_profiles: HashMap<TargetCategory, GateParams>,
    pub default_gate: GateParams,
    pub cooperative_id_boost: f64,
    pub cross_sensor_penalty: f64,
}

pub struct GateParams {
    pub position_gate_m: f64,      // max position residual in meters
    pub velocity_gate_mps: f64,    // max velocity residual in m/s
    pub chi_squared_threshold: f64, // Mahalanobis distance threshold
}

// Example gate profiles:
// FixedWing:      position=5000m, velocity=100m/s, chi_sq=16.27 (99.9%, 3 DOF)
// Drone:          position=500m,  velocity=30m/s,  chi_sq=11.34
// Person:         position=50m,   velocity=3m/s,   chi_sq=11.34
// SurfaceVessel:  position=2000m, velocity=15m/s,  chi_sq=9.21
// Missile:        position=10000m, velocity=500m/s, chi_sq=16.27
```

When the target category is unknown, the associator uses the widest (most permissive) gate and narrows once classification is established.

### Spatial Index for Gating Pre-filter

Without spatial pre-filtering, gating computes Mahalanobis distance for every (observation, track) pair: O(N x M). With 100 unassociated observations and 1,000 tracks, that's 100,000 innovation computations per cycle, each involving a 6x6 matrix multiply and inversion. At 10 Hz, that's 1,000,000 matrix operations/second just for gating - most of which are wasted on geographically impossible pairs.

A grid-based spatial index reduces this to O(N x k) where k is the number of tracks in nearby cells (typically 5-20 instead of 1,000):

```rust
pub struct SpatialIndex {
    /// Grid cells keyed by (lat_bin, lon_bin)
    grid: HashMap<(i32, i32), Vec<TrackId>>,
    cell_size_deg: f64,  // e.g., 0.5 degrees (~50km)
}

impl SpatialIndex {
    /// Update track position in the index (called after fusion update)
    pub fn update_track(&mut self, track_id: &TrackId, lat: f64, lon: f64);

    /// Remove a track from the index
    pub fn remove_track(&mut self, track_id: &TrackId);

    /// Find tracks in the same cell + 8 neighbors as an observation position
    pub fn nearby_tracks(&self, lat: f64, lon: f64) -> Vec<&TrackId>;
}
```

The index uses a 2D lat/lon grid (not 3D). Mixed-altitude targets (aircraft over a surface vessel) correctly appear in the same geographic cell - the altitude difference causes the Mahalanobis gate to reject the pair, which is correct behavior. Cell size of ~0.5 degrees (~50km) handles all target types - even at Mach 2, a target moves only 68m between 10 Hz association cycles.

Boundary handling: `nearby_tracks()` checks the current cell + all 8 neighbors to avoid missing targets near cell edges.

### GNN Implementation (Initial)

1. **Spatial pre-filter** - For each unassociated observation, query the spatial index for nearby tracks. Only these pairs proceed to gating. Reduces candidate pairs by >99% in typical scenarios.
2. **Gating** - For each candidate (observation, track) pair, compute Mahalanobis distance using `TrackerState::innovation()`. Select gate threshold based on track's `TargetClassification`. Exclude pairs exceeding threshold. Produces sparse cost matrix.
3. **Cost matrix** - Surviving pairs scored by Mahalanobis distance, discounted by cooperative ID match (ICAO/MMSI/etc. agreement is nearly deterministic).
4. **Optimal assignment** - Jonker-Volgenant algorithm [7] for global minimum-cost 1:1 assignment.

### Cooperative ID Fast Path

When an observation carries a cooperative ID matching a track's `cooperative_ids` list, the cost is heavily discounted. The gate still validates spatially (guards against ID spoofing), but the match is effectively deterministic. This works for ICAO (ADS-B), MMSI (AIS), NORAD ID (space surveillance), and any other cooperative identifier.

### Association System

```rust
fn association_system(
    config: Res<AssociatorConfig>,
    associator: Res<ActiveAssociator>,
    mut store: ResMut<TimelineStore>,
    tracks: Query<(Entity, &Track, &TrackerState, &TargetClassification)>,
) {
    let unassociated = store.unassociated();
    let track_list: Vec<_> = tracks.iter().collect();

    let result = associator.associate(&unassociated, &track_list, &config);

    // Associate matched observations in the store
    for assignment in &result.assignments {
        let track_id = &track_list[assignment.track_idx].1.id;
        store.associate(assignment.observation_idx, track_id);
    }
    // Unassigned observations remain in store.unassociated()
    // for track_initiation_system to evaluate
}
```

## NATS/JetStream Transport

### Subject Hierarchy

```
fusion.{tier}.{node_id}.tracks              # fused track updates
fusion.{tier}.{node_id}.tracks.{track_id}   # individual track
fusion.{tier}.{node_id}.observations         # raw observations (optional)
fusion.{tier}.{node_id}.status               # node heartbeat
fusion.{tier}.{node_id}.control              # commands
```

### Configuration

```rust
pub struct NatsTransportConfig {
    pub server_url: String,
    pub node_id: String,
    pub tier: FusionTier,
    pub publish_interval: Duration,
    pub subscriptions: Vec<SubConfig>,
    pub jetstream: JetStreamConfig,
}

pub struct JetStreamConfig {
    pub stream_name: String,
    pub retention: RetentionPolicy,
    pub max_age: Duration,
    pub max_bytes: u64,
    pub replicas: u8,
    pub consumer_deliver_policy: DeliverPolicy,
}
```

### DIL Resilience via JetStream

- **Persistent storage** - messages survive broker restarts and subscriber disconnections
- **Replay on reconnect** - missed messages delivered via `DeliverPolicy::ByStartTime`
- **Backpressure** - bounded streams prevent memory exhaustion on edge hardware
- **Acknowledgment** - consumer acks ensure delivery

Late messages replayed from JetStream are handled by the OOSM subsystem in the filter (rollback-and-replay from state history).

### Wire Format

Protobuf for fused track messages. State vector uses a variable-length representation to accommodate different state dimensionalities:

```protobuf
message FusedTrackUpdate {
    string track_id = 1;
    string node_id = 2;
    FusionTier tier = 3;
    google.protobuf.Timestamp timestamp = 4;
    FusedStateVector state = 5;
    CovarianceMatrix covariance = 6;
    TrackStatus status = 7;
    TargetClassificationProto classification = 8;
    repeated CooperativeId cooperative_ids = 9;
    float confidence = 10;
    uint32 sensor_count = 11;
    repeated string contributing_sensors = 12;
}

message FusedStateVector {
    StateVectorType type = 1;
    repeated double values = 2;  // variable length based on type
}

message CovarianceMatrix {
    uint32 dimension = 1;
    repeated double values = 2;  // row-major upper triangle
}

message TargetClassificationProto {
    TargetDomain domain = 1;
    TargetCategoryProto category = 2;
    optional string specific_type = 3;
    Affiliation affiliation = 4;
    float classification_confidence = 5;
}

message CooperativeId {
    TargetDomain domain = 1;
    IdentifierTypeProto id_type = 2;
    string id = 3;
}

enum StateVectorType {
    STATE_VECTOR_TYPE_UNSPECIFIED = 0;
    CARTESIAN_6DOF = 1;    // [x, y, z, vx, vy, vz] in ECEF
    SURFACE_4DOF = 2;      // [x, y, vx, vy] in ECEF (constrained to ellipsoid)
    MANEUVERING_9DOF = 3;  // [x, y, z, vx, vy, vz, ax, ay, az]
    // ORBITAL_6DOF = 4;   // future: Keplerian elements
}

enum FusionTier {
    FUSION_TIER_UNSPECIFIED = 0;
    EDGE = 1;
    REGIONAL = 2;
    GLOBAL = 3;
}

enum TrackStatus {
    TRACK_STATUS_UNSPECIFIED = 0;
    TENTATIVE = 1;
    CONFIRMED = 2;
    COASTING = 3;
    LOST = 4;
}

enum TargetDomain { /* mirrors Rust enum */ }
enum TargetCategoryProto { /* mirrors Rust enum */ }
enum Affiliation { /* mirrors Rust enum */ }
enum IdentifierTypeProto { /* mirrors Rust enum */ }
```

### Transport Systems

- `nats_publish_system` - publishes local fused tracks on a timer
- `nats_subscribe_system` - receives upstream tracks, inserts as `SensorObservation` with `Measurement::FusedEstimate`

Uses crossbeam channel to bridge async NATS to synchronous Bevy systems (same pattern as the existing ADS-B client).

### Offline Mode

When NATS is unreachable, the transport degrades gracefully - no publishing/subscribing, local fusion continues independently. Reconnection is automatic via `async-nats` built-in retry.

## AirJedi Integration Layer

A thin glue layer in the app between the fusion crate and existing rendering. The render bridge dispatches to domain-specific rendering strategies based on `TargetClassification`.

### Module Structure

```
src/fusion_integration/
+-- mod.rs              FusionIntegrationPlugin
+-- adsb_adapter.rs     ADS-B -> SensorObservation
+-- render_bridge.rs    TrackerState -> domain-specific visuals
+-- renderers/
|   +-- mod.rs          TrackRenderer trait, renderer registry
|   +-- aircraft.rs     Aircraft rendering (3D models, altitude coloring)
|   +-- surface.rs      Ship/ground vehicle rendering (2D icons)
|   +-- person.rs       Person/small target rendering
|   +-- space.rs        Satellite/debris rendering (orbital arcs)
|   +-- unknown.rs      Generic dot for unclassified targets
+-- uncertainty_viz.rs  Coasting ellipse rendering
+-- symbology.rs        MIL-STD-2525D / APP-6D symbol selection
+-- fusion_ui.rs        Track status in panels, sensor badges
```

### Render Bridge (Domain-Generic)

The render bridge reads `TrackerState` and `TargetClassification` and dispatches to the appropriate renderer. No assumption that everything is an aircraft:

```rust
fn sync_tracks_to_visuals(
    tracks: Query<
        (&Track, &TrackerState, &TrackQuality, &TargetClassification),
        Changed<TrackerState>,
    >,
    renderer_registry: Res<RendererRegistry>,
    // ... model registry, coordinate converter, etc.
) {
    for (track, tracker, quality, classification) in &tracks {
        let renderer = renderer_registry.get(classification.domain, classification.category);
        let (lat, lon, alt) = tracker.position_geodetic();

        match quality.status {
            TrackStatus::Confirmed => renderer.render_confirmed(...),
            TrackStatus::Tentative => renderer.render_tentative(...),
            TrackStatus::Coasting  => renderer.render_coasting(...),
            TrackStatus::Lost      => renderer.render_lost(...),
        }
    }
}
```

### Renderer Trait

```rust
pub trait TrackRenderer: Send + Sync + 'static {
    fn render_confirmed(&self, ...);    // normal visual
    fn render_tentative(&self, ...);    // partial/dotted visual
    fn render_coasting(&self, ...);     // dimmed + uncertainty ellipse
    fn render_lost(&self, ...);         // fade out
    fn despawn(&self, ...);             // clean up visual entities
}
```

Per-domain rendering behavior:
- **Aircraft** - 3D model + altitude coloring + trails (existing AirJedi visuals)
- **Ships** - 2D icon on water surface + heading indicator + wake trail
- **Ground vehicles** - 2D icon constrained to terrain + direction indicator
- **People** - small dot/icon, no trail, short coasting display
- **Satellites** - orbital path arc (future: ground track projection)
- **Missiles/rockets** - fast-fading trail, threat coloring, velocity vector
- **Drones** - smaller aircraft icon, possibly swarm visualization for groups
- **Unknown** - generic dot with uncertainty ring, upgrades when classified

### Symbology

For tactical applications, targets can optionally be rendered using standard military symbology (MIL-STD-2525D [9] or NATO APP-6D [10]) based on `TargetClassification` domain/category/affiliation. This is a display option, not the default for the civilian AirJedi use case.

### What Stays the Same

- Camera, tiles, UI panel framework, toolbar, status bar
- Keyboard shortcuts, picking, follow mode
- The aircraft renderer reuses existing 3D models, labels, trails, altitude coloring

### What Changes

- `sync_aircraft_from_adsb` replaced by adapter + generic render bridge
- `Aircraft` component becomes one renderer's output, not the universal track component
- `InterpolationState` smooths filter predictions instead of doing its own dead-reckoning
- `DataSourceManager` retired (fusion pipeline subsumes its role)
- New UI: track status indicators, sensor contribution badges, uncertainty visualization, target classification display

### System Scheduling: FixedUpdate vs Update

**Critical performance decision:** The fusion pipeline runs in Bevy's `FixedUpdate` schedule at 10-20 Hz, NOT per-frame in `Update` at 60fps. Display interpolation and rendering run per-frame.

**Why:** With 1,000 tracks, running `predict(dt)` every frame means 60,000 6x6 matrix operations/second. But observations arrive at 1-12 Hz per sensor - so 50-59 out of 60 frames, there are zero new observations for a given track. The prediction steps produce negligible state change (sub-millisecond dt) and waste CPU. Moving to 10-20 Hz reduces filter math to 10,000-20,000 matrix ops/second with no accuracy loss (process noise Q scales linearly with dt for constant-velocity models).

This is standard practice in game engines for physics simulation - and tracking IS physics. Real tracking systems also operate at fixed update rates matched to sensor rates, not display rates.

**Estimated FixedUpdate budget at 10Hz with 1,000 tracks:**
- Drain ~100-500 buffered observations: < 0.1ms
- Association with spatial index: ~100 obs x 20 nearby tracks = 2,000 gating ops: ~0.2ms
- Fusion predict on 1,000 tracks + ~200 updates: ~0.7ms
- Lifecycle scan of 1,000 tracks: ~0.1ms
- Total: **< 2ms per tick** (budget: 50-100ms)

### Ingest Buffer Pattern

Background I/O threads (ADS-B TCP, NATS async, future radar) accumulate data in thread-safe buffers (`Arc<Mutex<Vec<SensorObservation>>>`). Bevy ingest systems drain from these buffers using `try_lock` (non-blocking, same pattern the existing ADS-B client uses). Each ingest system writes to its own per-sensor buffer resource - no contention between ingest systems. A single drain system then merges all buffers into the `TimelineStore` with exclusive access.

This avoids the Bevy `ResMut` contention problem: multiple systems cannot hold `ResMut<TimelineStore>` simultaneously, which would serialize the "parallel" IngestSet. By having each ingest system write to its own buffer, true parallelism is preserved.

```rust
/// Per-sensor-type observation buffer (one per ingest system)
#[derive(Resource)]
pub struct SensorBuffer<T: SensorSource> {
    observations: Vec<SensorObservation>,
}
```

### System Sets

```
FixedUpdate (10-20 Hz) - all fusion work:
  IngestSet (parallel - each writes to own SensorBuffer resource):
    adsb_ingest_system       (drains from Arc<Mutex<>> ADS-B background thread)
    nats_ingest_system       (drains from Arc<Mutex<>> NATS background thread)
    ais_ingest_system        (future)
    radar_ingest_system      (future)

  -> DrainSet (exclusive TimelineStore access):
       drain_all_buffers_into_store

    -> AssociationSet:
         association_system
         update_spatial_index

      -> FusionSet:
           fusion_update_system

        -> LifecycleSet:
             track_status_system
             track_initiation_system
             track_classification_system
             track_merge_system
             track_cleanup_system

Update (60fps) - display only:
  InterpolationSet:
    interpolate_track_display_positions  (smooth between filter states)

  -> RenderBridgeSet:
       sync_tracks_to_visuals

    -> ExistingRenderingSets:
         labels, trails, models, etc.
```

The display interpolation system blends between the two most recent `TrackerState` snapshots for smooth per-frame rendering. This reuses the pattern from AirJedi's existing `InterpolationState` system.

## Testing Strategy

### Unit Tests (in airjedi-fusion)

- **Coordinate conversion** - ECEF/geodetic/ENU round-trips, known-position verification against reference values
- **Filter math** - EKF predict/update in ECEF against analytical solutions, outlier rejection, divergence recovery, OOSM rollback-and-replay
- **IMM** - model probability transitions (constant velocity -> coordinated turn -> back)
- **Associator** - GNN correctness: well-separated tracks, swapped-distance scenarios, cooperative ID override, mixed target types with different gates
- **Datastore** - Insert/query/association/eviction, Parquet round-trip fidelity
- **Track lifecycle** - State machine transitions per target category, merge behavior
- **Classification** - Category inference from kinematics

### Integration Tests (recorded data replay)

- **Single-source baseline** - Replay recorded ADS-B through pipeline, verify fused output matches raw input (one sensor = smoothed pass-through)
- **Simulated multi-sensor** - Synthetic ADS-B + radar for known flight paths, inject dropouts, verify coast-on-radar, verify noise weighting, compare against ground truth
- **Simulated multi-domain** - Mixed aircraft + surface vessel + ground vehicle scenario, verify correct filter selection and independent tracking
- **DIL transport** - Two fusion instances with NATS, verify cross-tier ingestion, disconnect/reconnect/replay, OOSM handling of replayed late messages
- **OOSM stress test** - Deliberately delay 20% of observations by 5-15 seconds, verify filter convergence matches the in-order case within tolerance

### Visual Verification (BRP)

- Inspect `TrackerState` and `TrackQuality` via BRP queries at runtime
- Screenshot comparisons for uncertainty ellipse rendering during coast
- Simulate ADS-B dropout by pausing adapter, observe visual state transitions
- Verify domain-specific rendering (different icons/trails per target category)

### Test Data

- `testdata/` directory with small anonymized Parquet recordings
- Synthetic trajectory generators (parameterized by waypoints, speed, turn rate, target category)
- Mixed-domain scenario generator (aircraft + ships + ground vehicles)
- Configurable sensor noise models (accuracy/latency per sensor type)

## Performance and Scaling Considerations

### Design Point

The system is designed and validated for: **200-500 sensor sources, 1,000 tracked targets, 2,000-5,000 observations/second, on a UI node rendering at 60fps.** The architectural decisions above (FixedUpdate scheduling, VecDeque hot buffer, spatial index, ingest buffer pattern) keep the fusion pipeline under 2ms per 10Hz tick at this scale.

### Memory Budget (Steady State at 1,000 Tracks)

| Component | Estimate |
|-----------|----------|
| TimelineStore hot buffer (60s retention, 5K obs/sec, ~450 bytes/obs) | ~135 MB |
| TrackerState per track (6x6 state + covariance + history) | ~1 MB |
| SpatialIndex (1,000 tracks in grid) | < 1 MB |
| Total fusion pipeline | ~140 MB |

The hot buffer dominates. If memory is constrained (edge nodes), reduce `hot_retention` from 60s to 10-15s.

### CPU Budget (Per FixedUpdate Tick at 10Hz)

| Stage | Operations | Estimate |
|-------|-----------|----------|
| Drain observation buffers | 100-500 inserts | < 0.1ms |
| Spatial index query | 100 obs x cell lookup | < 0.1ms |
| Association gating | 100 obs x ~20 nearby tracks = 2,000 innovations | ~0.2ms |
| Jonker-Volgenant assignment | ~50x50 sparse matrix | < 0.1ms |
| Fusion predict (1,000 tracks) | 1,000 x 6x6 matrix multiply | ~0.5ms |
| Fusion update (~200 tracks with new obs) | 200 x EKF update | ~0.2ms |
| Track lifecycle scan | 1,000 status checks | < 0.1ms |
| **Total** | | **< 1.5ms** |

Headroom: ~48ms remaining per tick at 10Hz. Scaling to 10,000 tracks would use ~15ms - still comfortable.

### Deferred Optimizations

These are noted for future implementation when profiling indicates they're needed:

1. **Stack-allocated covariance matrices.** Currently `DMatrix<f64>` (heap-allocated, dynamic size). For known sensor types, the covariance dimension is always the same (3x3, 6x6). Using `SMatrix` (stack-allocated, compile-time size) or a matrix pool would eliminate 5,000+ heap allocations/second. Deferred because modern allocators handle this rate without measurable overhead.

2. **Cross-sensor pre-grouping.** When multiple sensors of the same type report the same cooperative target (e.g., 5 ADS-B receivers all reporting the same ICAO), the observations can be pre-grouped by cooperative ID before entering the association pipeline. This reduces the association matrix size but NOT the number of filter updates (the filter still sees all observations). Framed as pre-grouping, not deduplication - discarding observations would lose information the filter can use.

3. **SIMD-accelerated matrix math.** nalgebra supports SIMD on x86_64. For large track counts, enabling `nalgebra`'s `simd` feature and ensuring 6x6 matrices are aligned could provide 2-4x speedup on filter math. Profile first.

4. **Parallel fusion across tracks.** The fusion update system iterates tracks sequentially. Since each track's filter is independent, this could be parallelized using Bevy's `par_iter()` or rayon. Only worth it above ~5,000 tracks where the sequential cost exceeds a few milliseconds.

## Bevy 0.19 Migration Sequencing

Bevy 0.19 was released on 2026-06-18. The migration affects AirJedi significantly but has minimal impact on the fusion crate. The recommended implementation sequence:

1. **Implement Plan 1 (fusion core crate) on Bevy 0.18** - The fusion crate uses only `bevy_app`, `bevy_ecs`, `bevy_time`, `bevy_reflect`, `bevy_log`, which have few breaking changes in 0.19. The main 0.19 change ("Resources as Components") is a non-issue as long as no type derives both `Component` and `Resource` (the design already avoids this).

2. **Implement Plan 2 (NATS transport) on Bevy 0.18** - No Bevy API surface touched by 0.19 changes.

3. **Migrate AirJedi to Bevy 0.19** - Wait for ecosystem dependencies to publish 0.19-compatible versions:
   - `bevy_egui` 0.40 - already available
   - `bevy_brp_extras` 0.19 - already available
   - `bevy-inspector-egui` - not yet updated (latest 0.36 targets 0.18)
   - `bevy_hanabi` - not yet updated (latest 0.18 targets Bevy 0.18)
   - `bevy_obj` - not yet updated (latest 0.18.2 targets Bevy 0.18)
   - `bevy_slippy_tiles` - local fork, must be updated manually

4. **Update `airjedi-fusion` Cargo.toml to Bevy 0.19** - Adjust for Resources-as-Components changes and `bevy_reflect` submodule reorganization. Expected to be a small diff.

5. **Implement Plan 3 (AirJedi integration) on Bevy 0.19** - This plan is heavily affected by 0.19 changes (`SceneRoot` -> `WorldAssetRoot`, Atmosphere as entity, Skybox moved to `bevy_light`, UI feature no longer implied by 3d/2d).

### Key 0.19 Changes Affecting This Design

| Change | Impact on Fusion Crate | Impact on AirJedi Integration |
|--------|----------------------|------------------------------|
| Resources as Components (`Resource` subtrait of `Component`) | Low - don't dual-derive, avoid broad queries with resources | Medium - audit existing broad queries |
| `SceneRoot` -> `WorldAssetRoot` | None | High - all 3D model loading |
| Atmosphere is now an Entity | None | High - `view3d/sky.rs` rework |
| `bevy_reflect` submodule reorg | Low - update import paths | Low |
| `bevy_scene` -> `bevy_world_serialization` | None | Medium - scene loading code |
| `World::clear_entities` now clears resources | Low - check test code | Low |

### Design Constraints for 0.19 Compatibility

To minimize friction when migrating the fusion crate to 0.19:
- Never derive both `Component` and `Resource` on the same type (already enforced)
- Don't use `World::clear_entities` in tests (use targeted entity despawning instead)
- Don't use broad queries (`Query<EntityMut>`) alongside `Res<T>` in the same system
- Use `uuid` crate API that's compatible with the version Bevy 0.19 depends on

## References

1. **Bar-Shalom, Y., Li, X.R., Kirubarajan, T.** *Estimation with Applications to Tracking and Navigation.* Wiley, 2001. The definitive reference for Kalman filtering in tracking systems. Chapter 6 covers coordinate system selection and why ECEF/ENU is preferred over geodetic for filter state.
   - Anti-pattern: running EKF in geodetic (lat/lon) coordinates mixes radians and meters in the covariance matrix, causing numerical conditioning failures.
   - https://onlinelibrary.wiley.com/doi/book/10.1002/0471221279

2. **Coordinate Systems for Tracking.** The standard practice in military and ATC tracking systems (STANAG 4586, MIL-STD-6016 Link 16) is to maintain filter state in ECEF or local ENU and convert to geodetic only for display.
   - ECEF overview: https://en.wikipedia.org/wiki/Earth-centered,_Earth-fixed_coordinate_system
   - ENU overview: https://en.wikipedia.org/wiki/Local_tangent_plane_coordinates

3. **Blom, H.A.P. and Bar-Shalom, Y.** *The Interacting Multiple Model Algorithm for Systems with Markovian Switching Coefficients.* IEEE Transactions on Automatic Control, 1988. The foundational paper on IMM for tracking maneuvering targets with mixed dynamics.
   - Essential when tracking targets that switch between motion regimes (cruise/turn, hover/transit, surface/dive).
   - https://ieeexplore.ieee.org/document/9101
   - IMM overview: https://en.wikipedia.org/wiki/Multiple_model

4. **Julier, S.J. and Uhlmann, J.K.** *A Non-divergent Estimation Algorithm in the Presence of Unknown Correlations.* IEEE ACC, 1997. Covariance Intersection - a conservative fusion method for combining estimates when cross-correlations are unknown, which is always the case in decentralized multi-tier architectures.
   - https://ieeexplore.ieee.org/document/609105
   - Overview: https://en.wikipedia.org/wiki/Covariance_intersection

5. **Bar-Shalom, Y.** *Update with Out-of-Sequence Measurements in Tracking.* IEEE Transactions on Aerospace and Electronic Systems, 2002. The standard algorithm for handling delayed/out-of-order observations, critical for any system with network latency or DIL conditions.
   - https://ieeexplore.ieee.org/document/1145846

6. **Bevy ECS Best Practices.** Components that must stay synchronized should be combined into a single component rather than split across multiple. Bevy queries guarantee atomic access to all components on an entity within a single system, but separate components can be independently modified by different systems, creating sync hazards.
   - https://bevy-cheatbook.github.io/programming/ec.html
   - Bevy component design: https://bevyengine.org/learn/quick-start/getting-started/ecs/

7. **Jonker, R. and Volgenant, A.** *A Shortest Augmenting Path Algorithm for Dense and Sparse Linear Assignment Problems.* Computing, 1987. The JV algorithm for optimal 1:1 assignment used in GNN track association. O(n^3) but fast in practice with sparse gating.
   - https://link.springer.com/article/10.1007/BF02278710
   - Rust crate: https://crates.io/crates/lapjv

8. **Rerun.io Data Model.** Immutable, append-only, multi-timeline data storage for sensor data. Key patterns: dual timeline indexing (sensor_time vs. log_time), columnar Arrow storage, entity-path hierarchies.
   - Architecture: https://rerun.io/docs/concepts/batches-and-caching
   - Data model: https://rerun.io/docs/concepts/entity-component

9. **MIL-STD-2525D.** Joint Military Symbology standard for tactical display. Defines symbols for air, ground, maritime, space, and subsurface targets with affiliation (friendly/hostile/neutral/unknown) encoding.
   - Overview: https://en.wikipedia.org/wiki/NATO_Joint_Military_Symbology

10. **NATO APP-6D.** NATO equivalent of MIL-STD-2525D for allied interoperability. Defines the same symbol set with minor differences.
    - Overview: https://en.wikipedia.org/wiki/NATO_Joint_Military_Symbology

11. **Mahalanobis Distance for Gating.** Statistical distance metric that accounts for covariance structure. Used to determine if an observation is statistically compatible with a track's predicted state. Gate threshold set from chi-squared distribution with degrees of freedom equal to measurement dimension.
    - https://en.wikipedia.org/wiki/Mahalanobis_distance
    - Chi-squared table for gating: 3 DOF, 99% = 11.34; 3 DOF, 99.9% = 16.27

12. **NATS JetStream for DIL.** JetStream adds persistence, replay, and guaranteed delivery to NATS pub/sub, making it suitable for Disconnected, Intermittent, Limited bandwidth (DIL) environments.
    - https://docs.nats.io/nats-concepts/jetstream
    - Rust client: https://crates.io/crates/async-nats

13. **Bevy FixedUpdate for Physics/Simulation.** Bevy's `FixedUpdate` schedule runs at a fixed timestep independent of frame rate. Standard practice for physics, networking, and simulation systems that should not be coupled to display refresh rate. Filter predict/update is a simulation, not a rendering concern.
    - https://bevyengine.org/learn/quick-start/getting-started/ecs/#schedules
    - https://bevy-cheatbook.github.io/fundamentals/fixed-timestep.html

14. **Arrow/Parquet for Analytics, Not Hot Paths.** Apache Arrow is a columnar in-memory format designed for batch analytics - arrays are immutable once built. Using Arrow for a hot ring buffer with constant appends/evictions adds unnecessary overhead. Best practice: use plain Rust data structures for hot paths, Arrow as the serialization format for cold storage and inter-process exchange.
    - https://arrow.apache.org/docs/format/Columnar.html
    - Rust crate: https://crates.io/crates/arrow

15. **Spatial Indexing for Track Association.** Grid-based or R-tree spatial indexing reduces association gating from O(N x M) to O(N x k) by pre-filtering geographically impossible pairs. Standard in multi-target tracking systems. Grid-based is simplest and sufficient for targets with bounded velocity.
    - R-tree overview: https://en.wikipedia.org/wiki/R-tree
    - Geohash for spatial binning: https://en.wikipedia.org/wiki/Geohash
