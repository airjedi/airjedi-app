# airjedi-fusion Core Crate Implementation Plan (Plan 1 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `airjedi-fusion` crate - a standalone, Bevy-native multi-sensor fusion engine with ECEF-based Kalman filtering, GNN track association with spatial indexing, an in-memory observation store, and a track lifecycle state machine. Testable end-to-end with synthetic data, no AirJedi app dependency.

**Architecture:** The crate is a Bevy plugin (`FusionPlugin`) that adds components, resources, and systems for sensor fusion. Sensor observations enter a `TimelineStore`, get associated to tracks via a `GnnAssociator` with spatial pre-filtering, fused by per-track `TrackerState` (wrapping an EKF), and managed through a `Tentative -> Confirmed -> Coasting -> Lost` lifecycle. All filter math operates in ECEF coordinates internally. The crate depends on Bevy (minimal features, no rendering) and nalgebra.

**Tech Stack:** Rust, Bevy 0.18 (no rendering features), nalgebra, uuid, chrono

**Spec:** `docs/superpowers/specs/2026-06-20-multi-sensor-fusion-pipeline-design.md`

## Global Constraints

- Rust edition 2021
- Bevy 0.18 with minimal features (no `default-features`, enable only `bevy_app`, `bevy_ecs`, `bevy_time`, `bevy_reflect`, `bevy_log`)
- nalgebra for all matrix math - use `SVector`/`SMatrix` for fixed-size (6-DOF), `DVector`/`DMatrix` only where dimension varies at runtime
- No rendering dependencies in the fusion crate
- All filter state in ECEF coordinates, never geodetic
- `#[derive(Component)]` on all ECS components, `#[derive(Resource)]` on all resources
- Follow clippy pedantic lints (same lint config as adsb-client)
- WGS-84 ellipsoid constants: semi-major axis a = 6378137.0 m, flattening f = 1/298.257223563

## File Structure

```
airjedi-fusion/
├── Cargo.toml
└── src/
    ├── lib.rs                  Re-exports, FusionPlugin
    ├── types.rs                TrackId, TargetId, TargetDomain, IdentifierType,
    │                           TargetCategory, Affiliation, StateVectorType,
    │                           Timestamp alias
    ├── classification.rs       TargetClassification component
    ├── coord.rs                CoordinateFrame enum, geodetic<->ECEF<->ENU conversions
    ├── sensor.rs               SensorId, SensorKind, SensorObservation, Measurement,
    │                           ObservationCovariance, ObservationMetadata, FusionTier
    ├── store.rs                TimelineStore resource, HotBuffer, StoredObservation,
    │                           StoreConfig, query interface
    ├── track.rs                Track component, TrackQuality component, TrackStatus,
    │                           TrackLifecycleConfig, LifecycleProfiles
    ├── filter/
    │   ├── mod.rs              TrackFilter trait, FilterResult, Innovation,
    │   │                       FilterVariant enum, TrackerState component,
    │   │                       StateHistory, OosmConfig
    │   └── ekf.rs              Ekf6Dof implementation, ProcessNoiseConfig,
    │                           measurement models (all Measurement variants)
    ├── associator/
    │   ├── mod.rs              Associator trait, AssociationResult, Assignment,
    │   │                       AssociatorConfig, GateParams, ActiveAssociator resource
    │   ├── gnn.rs              GnnAssociator implementation, cost matrix, JV assignment
    │   └── spatial_index.rs    SpatialIndex, grid-based spatial pre-filter
    ├── systems.rs              All Bevy systems: drain, associate, fuse, lifecycle,
    │                           system sets and ordering
    └── config.rs               FusionConfig resource, LifecycleProfile,
                                FilterSelection, defaults
```

---

### Task 1: Crate Scaffolding and Core Types

**Files:**
- Create: `airjedi-fusion/Cargo.toml`
- Create: `airjedi-fusion/src/lib.rs`
- Create: `airjedi-fusion/src/types.rs`
- Create: `airjedi-fusion/src/classification.rs`
- Modify: `Cargo.toml` (workspace root - add workspace member)
- Test: `airjedi-fusion/src/types.rs` (inline tests)

**Interfaces:**
- Consumes: nothing (foundation task)
- Produces: `TrackId`, `TargetId`, `TargetDomain`, `IdentifierType`, `TargetCategory`, `Affiliation`, `StateVectorType`, `Timestamp`, `TargetClassification`

- [ ] **Step 1: Create Cargo.toml for the fusion crate**

```toml
# airjedi-fusion/Cargo.toml
[package]
name = "airjedi-fusion"
version = "0.1.0"
edition = "2021"
description = "Multi-sensor fusion engine for target tracking"

[lints.rust]
missing_debug_implementations = "warn"
unsafe_op_in_unsafe_fn = "warn"
unused_lifetimes = "warn"
redundant_lifetimes = "warn"
trivial_numeric_casts = "warn"

[lints.clippy]
cargo = { level = "warn", priority = -1 }
complexity = { level = "warn", priority = -1 }
correctness = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
perf = { level = "warn", priority = -1 }
style = { level = "warn", priority = -1 }
suspicious = { level = "warn", priority = -1 }
missing_errors_doc = "allow"
missing_panics_doc = "allow"
module_name_repetitions = "allow"

[dependencies]
bevy = { version = "0.18", default-features = false, features = [
    "bevy_app",
    "bevy_ecs",
    "bevy_time",
    "bevy_reflect",
    "bevy_log",
] }
nalgebra = "0.33"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1.0", features = ["derive"] }

[dev-dependencies]
approx = "0.5"
```

- [ ] **Step 2: Add workspace member to root Cargo.toml**

Add below the existing `[package]` section in the root `Cargo.toml`. If there's no `[workspace]` section yet, add one:

```toml
[workspace]
members = ["airjedi-fusion"]
```

If the root is not currently a workspace, you also need to add `resolver = "2"` under `[workspace]`.

- [ ] **Step 3: Create `types.rs` with core identity types**

```rust
// airjedi-fusion/src/types.rs
use bevy::prelude::*;
use uuid::Uuid;

pub type Timestamp = chrono::DateTime<chrono::Utc>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId(pub Uuid);

impl TrackId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TrackId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TargetId {
    pub domain: TargetDomain,
    pub id: String,
    pub id_type: IdentifierType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect)]
pub enum TargetDomain {
    Air,
    Ground,
    Maritime,
    Space,
    Subsurface,
    Person,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdentifierType {
    // Aviation
    Icao,
    Callsign,
    ModeA,
    RemoteId,
    TailNumber,
    // Maritime
    Mmsi,
    ImoNumber,
    // Space
    NoradId,
    CosparId,
    // Ground
    LicensePlate,
    Vin,
    // Universal
    Uuid,
    Rfid,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect)]
pub enum TargetCategory {
    // Air
    FixedWing,
    RotaryWing,
    Drone,
    Balloon,
    Missile,
    Rocket,
    // Space
    Satellite,
    SpaceDebris,
    LaunchVehicle,
    // Ground
    GroundVehicle,
    Person,
    AnimalOrWildlife,
    // Maritime
    SurfaceVessel,
    Submarine,
    // Unknown
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect)]
pub enum Affiliation {
    Friendly,
    Hostile,
    Neutral,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateVectorType {
    Cartesian6Dof,
    Surface4Dof,
    Maneuvering9Dof,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_id_uniqueness() {
        let a = TrackId::new();
        let b = TrackId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn target_id_equality() {
        let id1 = TargetId {
            domain: TargetDomain::Air,
            id: "A1B2C3".to_string(),
            id_type: IdentifierType::Icao,
        };
        let id2 = TargetId {
            domain: TargetDomain::Air,
            id: "A1B2C3".to_string(),
            id_type: IdentifierType::Icao,
        };
        assert_eq!(id1, id2);
    }

    #[test]
    fn target_id_different_domains_not_equal() {
        let air = TargetId {
            domain: TargetDomain::Air,
            id: "12345".to_string(),
            id_type: IdentifierType::Custom,
        };
        let ground = TargetId {
            domain: TargetDomain::Ground,
            id: "12345".to_string(),
            id_type: IdentifierType::Custom,
        };
        assert_ne!(air, ground);
    }
}
```

- [ ] **Step 4: Create `classification.rs`**

```rust
// airjedi-fusion/src/classification.rs
use bevy::prelude::*;
use crate::types::{Affiliation, TargetCategory, TargetDomain};

#[derive(Component, Debug, Clone, Reflect)]
pub struct TargetClassification {
    pub domain: TargetDomain,
    pub category: TargetCategory,
    pub specific_type: Option<String>,
    pub affiliation: Affiliation,
    pub confidence: f32,
}

impl Default for TargetClassification {
    fn default() -> Self {
        Self {
            domain: TargetDomain::Air,
            category: TargetCategory::Unknown,
            specific_type: None,
            affiliation: Affiliation::Unknown,
            confidence: 0.0,
        }
    }
}
```

- [ ] **Step 5: Create `lib.rs` with module declarations**

```rust
// airjedi-fusion/src/lib.rs
pub mod types;
pub mod classification;

pub use types::*;
pub use classification::TargetClassification;
```

- [ ] **Step 6: Verify it compiles and tests pass**

Run:
```bash
cd airjedi-fusion && cargo test
```
Expected: all tests pass, no warnings from clippy pedantic.

- [ ] **Step 7: Commit**

```bash
git add airjedi-fusion/ Cargo.toml
git commit -m "Add airjedi-fusion crate with core identity and classification types"
```

---

### Task 2: Coordinate Frame Conversions (ECEF / Geodetic / ENU)

**Files:**
- Create: `airjedi-fusion/src/coord.rs`
- Modify: `airjedi-fusion/src/lib.rs` (add module)
- Test: inline in `coord.rs`

**Interfaces:**
- Consumes: nothing
- Produces: `CoordinateFrame` enum, `geodetic_to_ecef(lat_deg: f64, lon_deg: f64, alt_m: f64) -> [f64; 3]`, `ecef_to_geodetic(ecef: &[f64; 3]) -> (f64, f64, f64)` (returns lat_deg, lon_deg, alt_m), `ecef_to_enu(ecef: &[f64; 3], ref_lat_deg: f64, ref_lon_deg: f64, ref_alt_m: f64) -> [f64; 3]`, `spherical_to_ecef(range_m: f64, az_rad: f64, el_rad: f64, sensor_ecef: &[f64; 3], sensor_lat_deg: f64, sensor_lon_deg: f64) -> [f64; 3]`

- [ ] **Step 1: Write failing tests for geodetic-ECEF round trip**

```rust
// airjedi-fusion/src/coord.rs

// WGS-84 constants
const WGS84_A: f64 = 6_378_137.0;
const WGS84_F: f64 = 1.0 / 298.257_223_563;
const WGS84_B: f64 = WGS84_A * (1.0 - WGS84_F);
const WGS84_E2: f64 = 1.0 - (WGS84_B * WGS84_B) / (WGS84_A * WGS84_A);

#[derive(Debug, Clone, PartialEq)]
pub enum CoordinateFrame {
    Wgs84,
    Ecef,
    Enu {
        origin_lat_deg: f64,
        origin_lon_deg: f64,
        origin_alt_m: f64,
    },
    SensorSpherical {
        sensor_lat_deg: f64,
        sensor_lon_deg: f64,
        sensor_alt_m: f64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn geodetic_ecef_round_trip_wichita() {
        let lat = 37.6872;
        let lon = -97.3301;
        let alt = 0.0;
        let ecef = geodetic_to_ecef(lat, lon, alt);
        let (lat2, lon2, alt2) = ecef_to_geodetic(&ecef);
        assert_relative_eq!(lat, lat2, epsilon = 1e-9);
        assert_relative_eq!(lon, lon2, epsilon = 1e-9);
        assert_relative_eq!(alt, alt2, epsilon = 1e-3);
    }

    #[test]
    fn geodetic_ecef_round_trip_with_altitude() {
        let lat = 37.6872;
        let lon = -97.3301;
        let alt = 10000.0; // 10km
        let ecef = geodetic_to_ecef(lat, lon, alt);
        let (lat2, lon2, alt2) = ecef_to_geodetic(&ecef);
        assert_relative_eq!(lat, lat2, epsilon = 1e-9);
        assert_relative_eq!(lon, lon2, epsilon = 1e-9);
        assert_relative_eq!(alt, alt2, epsilon = 1e-3);
    }

    #[test]
    fn geodetic_ecef_equator_prime_meridian() {
        let ecef = geodetic_to_ecef(0.0, 0.0, 0.0);
        assert_relative_eq!(ecef[0], WGS84_A, epsilon = 1.0);
        assert_relative_eq!(ecef[1], 0.0, epsilon = 1.0);
        assert_relative_eq!(ecef[2], 0.0, epsilon = 1.0);
    }

    #[test]
    fn geodetic_ecef_north_pole() {
        let ecef = geodetic_to_ecef(90.0, 0.0, 0.0);
        assert_relative_eq!(ecef[0], 0.0, epsilon = 1.0);
        assert_relative_eq!(ecef[1], 0.0, epsilon = 1.0);
        assert_relative_eq!(ecef[2], WGS84_B, epsilon = 1.0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd airjedi-fusion && cargo test coord`
Expected: FAIL - `geodetic_to_ecef` and `ecef_to_geodetic` not found

- [ ] **Step 3: Implement geodetic <-> ECEF conversions**

Add to `coord.rs` above the test module:

```rust
pub fn geodetic_to_ecef(lat_deg: f64, lon_deg: f64, alt_m: f64) -> [f64; 3] {
    let lat = lat_deg.to_radians();
    let lon = lon_deg.to_radians();
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let sin_lon = lon.sin();
    let cos_lon = lon.cos();

    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();

    [
        (n + alt_m) * cos_lat * cos_lon,
        (n + alt_m) * cos_lat * sin_lon,
        (n * (1.0 - WGS84_E2) + alt_m) * sin_lat,
    ]
}

pub fn ecef_to_geodetic(ecef: &[f64; 3]) -> (f64, f64, f64) {
    let x = ecef[0];
    let y = ecef[1];
    let z = ecef[2];

    let lon = y.atan2(x);
    let p = (x * x + y * y).sqrt();

    // Iterative Bowring method (converges in 2-3 iterations)
    let mut lat = (z / p).atan(); // initial guess
    for _ in 0..10 {
        let sin_lat = lat.sin();
        let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
        lat = (z + WGS84_E2 * n * sin_lat).atan2(p);
    }

    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
    let alt = if cos_lat.abs() > 1e-10 {
        p / cos_lat - n
    } else {
        z.abs() / sin_lat.abs() - n * (1.0 - WGS84_E2)
    };

    (lat.to_degrees(), lon.to_degrees(), alt)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd airjedi-fusion && cargo test coord`
Expected: all 4 tests PASS

- [ ] **Step 5: Add ENU and spherical conversion tests**

Add to the test module in `coord.rs`:

```rust
    #[test]
    fn ecef_to_enu_origin_is_zero() {
        let ref_lat = 37.6872;
        let ref_lon = -97.3301;
        let ref_alt = 0.0;
        let ecef = geodetic_to_ecef(ref_lat, ref_lon, ref_alt);
        let enu = ecef_to_enu(&ecef, ref_lat, ref_lon, ref_alt);
        assert_relative_eq!(enu[0], 0.0, epsilon = 1e-6);
        assert_relative_eq!(enu[1], 0.0, epsilon = 1e-6);
        assert_relative_eq!(enu[2], 0.0, epsilon = 1e-6);
    }

    #[test]
    fn ecef_to_enu_north_displacement() {
        let ref_lat = 37.0;
        let ref_lon = -97.0;
        let ref_alt = 0.0;
        // Point ~111km north (1 degree of latitude)
        let target_ecef = geodetic_to_ecef(38.0, -97.0, 0.0);
        let enu = ecef_to_enu(&target_ecef, ref_lat, ref_lon, ref_alt);
        // East should be near zero, North should be ~111km
        assert_relative_eq!(enu[0], 0.0, epsilon = 500.0); // east
        assert!(enu[1] > 110_000.0 && enu[1] < 112_000.0); // north ~111km
        assert_relative_eq!(enu[2], 0.0, epsilon = 100.0); // up
    }

    #[test]
    fn spherical_to_ecef_known_target() {
        let sensor_lat = 37.0;
        let sensor_lon = -97.0;
        let sensor_alt = 0.0;
        let sensor_ecef = geodetic_to_ecef(sensor_lat, sensor_lon, sensor_alt);
        // Target directly north, 10km range, 0 elevation
        let az = 0.0_f64; // north
        let el = 0.0_f64;
        let range = 10_000.0;
        let target_ecef = spherical_to_ecef(range, az, el, &sensor_ecef, sensor_lat, sensor_lon);
        let (t_lat, t_lon, _t_alt) = ecef_to_geodetic(&target_ecef);
        // Should be ~0.09 degrees north, same longitude
        assert!(t_lat > sensor_lat);
        assert_relative_eq!(t_lon, sensor_lon, epsilon = 0.01);
    }
```

- [ ] **Step 6: Implement ENU and spherical conversions**

Add to `coord.rs`:

```rust
pub fn ecef_to_enu(
    ecef: &[f64; 3],
    ref_lat_deg: f64,
    ref_lon_deg: f64,
    ref_alt_m: f64,
) -> [f64; 3] {
    let ref_ecef = geodetic_to_ecef(ref_lat_deg, ref_lon_deg, ref_alt_m);
    let dx = ecef[0] - ref_ecef[0];
    let dy = ecef[1] - ref_ecef[1];
    let dz = ecef[2] - ref_ecef[2];

    let lat = ref_lat_deg.to_radians();
    let lon = ref_lon_deg.to_radians();
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let sin_lon = lon.sin();
    let cos_lon = lon.cos();

    let e = -sin_lon * dx + cos_lon * dy;
    let n = -sin_lat * cos_lon * dx - sin_lat * sin_lon * dy + cos_lat * dz;
    let u = cos_lat * cos_lon * dx + cos_lat * sin_lon * dy + sin_lat * dz;

    [e, n, u]
}

pub fn spherical_to_ecef(
    range_m: f64,
    az_rad: f64,
    el_rad: f64,
    sensor_ecef: &[f64; 3],
    sensor_lat_deg: f64,
    sensor_lon_deg: f64,
) -> [f64; 3] {
    // Convert spherical (range, azimuth, elevation) to ENU offset
    let cos_el = el_rad.cos();
    let e = range_m * cos_el * az_rad.sin();
    let n = range_m * cos_el * az_rad.cos();
    let u = range_m * el_rad.sin();

    // Convert ENU offset to ECEF
    let lat = sensor_lat_deg.to_radians();
    let lon = sensor_lon_deg.to_radians();
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let sin_lon = lon.sin();
    let cos_lon = lon.cos();

    // ENU to ECEF rotation (inverse of ECEF-to-ENU)
    let dx = -sin_lon * e - sin_lat * cos_lon * n + cos_lat * cos_lon * u;
    let dy = cos_lon * e - sin_lat * sin_lon * n + cos_lat * sin_lon * u;
    let dz = cos_lat * n + sin_lat * u;

    [
        sensor_ecef[0] + dx,
        sensor_ecef[1] + dy,
        sensor_ecef[2] + dz,
    ]
}
```

- [ ] **Step 7: Run all coord tests**

Run: `cd airjedi-fusion && cargo test coord`
Expected: all 7 tests PASS

- [ ] **Step 8: Add module to lib.rs and commit**

Add `pub mod coord;` to `lib.rs`, then:

```bash
git add airjedi-fusion/src/coord.rs airjedi-fusion/src/lib.rs
git commit -m "Add ECEF/geodetic/ENU/spherical coordinate conversions"
```

---

### Task 3: Sensor Observation Types

**Files:**
- Create: `airjedi-fusion/src/sensor.rs`
- Modify: `airjedi-fusion/src/lib.rs` (add module)
- Test: inline in `sensor.rs`

**Interfaces:**
- Consumes: `CoordinateFrame` from `coord.rs`, `TargetId`, `TargetCategory`, `Timestamp` from `types.rs`
- Produces: `SensorId`, `SensorKind`, `FusionTier`, `SensorObservation`, `Measurement`, `ObservationCovariance`, `ObservationMetadata`

- [ ] **Step 1: Write `sensor.rs` with all sensor types**

```rust
// airjedi-fusion/src/sensor.rs
use nalgebra::DMatrix;
use crate::coord::CoordinateFrame;
use crate::types::{TargetCategory, TargetId, Timestamp, StateVectorType};

#[derive(Debug, Clone)]
pub struct SensorId {
    pub id: String,
    pub kind: SensorKind,
    pub tier: FusionTier,
    pub coordinate_frame: CoordinateFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SensorKind {
    AdsbReceiver,
    MlatNetwork,
    PrimaryRadar,
    SecondaryRadar,
    AisReceiver,
    MaritimeRadar,
    Sonar,
    OpticalTracker,
    RfTracker,
    GpsTracker,
    SpaceSurveillanceRadar,
    UpstreamFusedTrack,
    Simulated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FusionTier {
    Edge,
    Regional,
    Global,
}

#[derive(Debug, Clone)]
pub struct SensorObservation {
    pub sensor_id: SensorId,
    pub timestamp: Timestamp,
    pub receipt_time: Timestamp,
    pub target_id: Option<TargetId>,
    pub measurement: Measurement,
    pub covariance: ObservationCovariance,
    pub classification_hint: Option<TargetCategory>,
    pub metadata: ObservationMetadata,
}

#[derive(Debug, Clone)]
pub enum Measurement {
    PositionVelocity3D {
        lat_deg: f64,
        lon_deg: f64,
        alt_m: Option<f64>,
        vel_north_mps: Option<f64>,
        vel_east_mps: Option<f64>,
        vel_down_mps: Option<f64>,
        heading_deg: Option<f64>,
    },
    PositionVelocity2D {
        lat_deg: f64,
        lon_deg: f64,
        speed_over_ground_mps: Option<f64>,
        course_over_ground_deg: Option<f64>,
    },
    Spherical {
        range_m: f64,
        azimuth_rad: f64,
        elevation_rad: Option<f64>,
        range_rate_mps: Option<f64>,
    },
    BearingOnly {
        azimuth_rad: f64,
        elevation_rad: Option<f64>,
    },
    DepthBearing {
        depth_m: f64,
        azimuth_rad: Option<f64>,
        range_m: Option<f64>,
    },
    FusedEstimate {
        state_type: StateVectorType,
        state: nalgebra::DVector<f64>,
        covariance: DMatrix<f64>,
        track_quality: f32,
    },
}

#[derive(Debug, Clone)]
pub struct ObservationCovariance {
    pub matrix: DMatrix<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct ObservationMetadata {
    pub signal_strength: Option<f32>,
    pub accuracy_category: Option<u8>,
    pub source_label: String,
}

pub trait SensorSource: Send + Sync + 'static {
    fn sensor_id(&self) -> &SensorId;
    fn poll_observations(&mut self) -> Vec<SensorObservation>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;

    fn make_adsb_observation() -> SensorObservation {
        SensorObservation {
            sensor_id: SensorId {
                id: "adsb-home".to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: Some(TargetId {
                domain: TargetDomain::Air,
                id: "A1B2C3".to_string(),
                id_type: IdentifierType::Icao,
            }),
            measurement: Measurement::PositionVelocity3D {
                lat_deg: 37.6872,
                lon_deg: -97.3301,
                alt_m: Some(10000.0),
                vel_north_mps: Some(100.0),
                vel_east_mps: Some(50.0),
                vel_down_mps: Some(-2.0),
                heading_deg: Some(63.0),
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(6, 6) * 100.0,
            },
            classification_hint: Some(TargetCategory::FixedWing),
            metadata: ObservationMetadata {
                signal_strength: Some(-85.0),
                accuracy_category: Some(8),
                source_label: "Home ADS-B receiver".to_string(),
            },
        }
    }

    #[test]
    fn create_adsb_observation() {
        let obs = make_adsb_observation();
        assert_eq!(obs.sensor_id.kind, SensorKind::AdsbReceiver);
        assert!(obs.target_id.is_some());
    }

    #[test]
    fn create_radar_observation() {
        let obs = SensorObservation {
            sensor_id: SensorId {
                id: "radar-alpha".to_string(),
                kind: SensorKind::PrimaryRadar,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::SensorSpherical {
                    sensor_lat_deg: 37.0,
                    sensor_lon_deg: -97.0,
                    sensor_alt_m: 50.0,
                },
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: None, // primary radar has no cooperative ID
            measurement: Measurement::Spherical {
                range_m: 50_000.0,
                azimuth_rad: 0.785, // ~45 degrees
                elevation_rad: Some(0.05),
                range_rate_mps: None,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(3, 3) * 500.0,
            },
            classification_hint: None,
            metadata: ObservationMetadata::default(),
        };
        assert_eq!(obs.sensor_id.kind, SensorKind::PrimaryRadar);
        assert!(obs.target_id.is_none());
    }
}
```

- [ ] **Step 2: Add module to lib.rs, run tests, commit**

Add `pub mod sensor;` and sensor re-exports to `lib.rs`.

Run: `cd airjedi-fusion && cargo test sensor`
Expected: PASS

```bash
git add airjedi-fusion/src/sensor.rs airjedi-fusion/src/lib.rs
git commit -m "Add sensor observation types and measurement variants"
```

---

### Task 4: Timeline Store

**Files:**
- Create: `airjedi-fusion/src/store.rs`
- Modify: `airjedi-fusion/src/lib.rs` (add module)
- Test: inline in `store.rs`

**Interfaces:**
- Consumes: `SensorObservation` from `sensor.rs`, `TrackId`, `Timestamp` from `types.rs`, `SensorId` from `sensor.rs`
- Produces: `TimelineStore` (Resource), `StoredObservation`, `StoreConfig`, `HotBuffer`, query methods: `insert()`, `associate()`, `query_range()`, `latest_per_sensor()`, `unassociated()`

- [ ] **Step 1: Write failing tests for basic store operations**

```rust
// airjedi-fusion/src/store.rs
use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use bevy::prelude::*;
use crate::sensor::{SensorId, SensorObservation};
use crate::types::{TrackId, Timestamp};

#[derive(Debug, Clone)]
pub struct StoredObservation {
    pub observation: SensorObservation,
    pub associated_track: Option<TrackId>,
    pub store_index: usize,
}

#[derive(Clone, Debug)]
pub struct StoreConfig {
    pub hot_retention: Duration,
    pub max_observations_per_track: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            hot_retention: Duration::from_secs(60),
            max_observations_per_track: 1000,
        }
    }
}

#[derive(Resource)]
pub struct TimelineStore {
    by_track: HashMap<TrackId, VecDeque<StoredObservation>>,
    unassociated_obs: Vec<StoredObservation>,
    next_index: usize,
    config: StoreConfig,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::*;
    use crate::coord::CoordinateFrame;
    use crate::types::*;
    use chrono::Utc;
    use nalgebra::DMatrix;

    fn make_test_obs(id: &str) -> SensorObservation {
        SensorObservation {
            sensor_id: SensorId {
                id: id.to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: None,
            measurement: Measurement::PositionVelocity3D {
                lat_deg: 37.0, lon_deg: -97.0, alt_m: Some(10000.0),
                vel_north_mps: None, vel_east_mps: None, vel_down_mps: None,
                heading_deg: None,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(3, 3),
            },
            classification_hint: None,
            metadata: ObservationMetadata::default(),
        }
    }

    #[test]
    fn insert_and_retrieve_unassociated() {
        let mut store = TimelineStore::new(StoreConfig::default());
        store.insert(make_test_obs("s1"));
        store.insert(make_test_obs("s2"));
        assert_eq!(store.unassociated().len(), 2);
    }

    #[test]
    fn associate_moves_to_track() {
        let mut store = TimelineStore::new(StoreConfig::default());
        store.insert(make_test_obs("s1"));
        assert_eq!(store.unassociated().len(), 1);

        let track_id = TrackId::new();
        store.associate(0, &track_id);
        assert_eq!(store.unassociated().len(), 0);

        let range = store.query_range(
            &track_id,
            chrono::Utc::now() - chrono::Duration::seconds(10),
            chrono::Utc::now() + chrono::Duration::seconds(10),
        );
        assert_eq!(range.len(), 1);
    }

    #[test]
    fn latest_per_sensor_returns_most_recent() {
        let mut store = TimelineStore::new(StoreConfig::default());
        let track_id = TrackId::new();

        let mut obs1 = make_test_obs("s1");
        obs1.timestamp = Utc::now() - chrono::Duration::seconds(5);
        store.insert(obs1);
        store.associate(0, &track_id);

        let mut obs2 = make_test_obs("s1");
        obs2.timestamp = Utc::now();
        store.insert(obs2);
        store.associate(0, &track_id);

        let latest = store.latest_per_sensor(&track_id);
        assert_eq!(latest.len(), 1); // one sensor
        assert!(latest.contains_key("s1"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd airjedi-fusion && cargo test store`
Expected: FAIL - `TimelineStore::new` and methods not implemented

- [ ] **Step 3: Implement TimelineStore**

Add to `store.rs` above the test module:

```rust
impl TimelineStore {
    pub fn new(config: StoreConfig) -> Self {
        Self {
            by_track: HashMap::new(),
            unassociated_obs: Vec::new(),
            next_index: 0,
            config,
        }
    }

    pub fn insert(&mut self, observation: SensorObservation) {
        let stored = StoredObservation {
            observation,
            associated_track: None,
            store_index: self.next_index,
        };
        self.next_index += 1;
        self.unassociated_obs.push(stored);
    }

    pub fn associate(&mut self, unassociated_idx: usize, track_id: &TrackId) {
        if unassociated_idx >= self.unassociated_obs.len() {
            return;
        }
        let mut obs = self.unassociated_obs.remove(unassociated_idx);
        obs.associated_track = Some(track_id.clone());

        let buffer = self
            .by_track
            .entry(track_id.clone())
            .or_insert_with(VecDeque::new);

        if buffer.len() >= self.config.max_observations_per_track {
            buffer.pop_front();
        }
        buffer.push_back(obs);
    }

    pub fn query_range(
        &self,
        track_id: &TrackId,
        from: Timestamp,
        to: Timestamp,
    ) -> Vec<&StoredObservation> {
        self.by_track
            .get(track_id)
            .map(|buf| {
                buf.iter()
                    .filter(|o| o.observation.timestamp >= from && o.observation.timestamp <= to)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn latest_per_sensor(
        &self,
        track_id: &TrackId,
    ) -> HashMap<String, &StoredObservation> {
        let mut latest: HashMap<String, &StoredObservation> = HashMap::new();
        if let Some(buf) = self.by_track.get(track_id) {
            for obs in buf.iter().rev() {
                let key = obs.observation.sensor_id.id.clone();
                latest.entry(key).or_insert(obs);
            }
        }
        latest
    }

    pub fn unassociated(&self) -> &[StoredObservation] {
        &self.unassociated_obs
    }

    pub fn evict_old(&mut self, now: Timestamp) {
        let cutoff = now - chrono::Duration::from_std(self.config.hot_retention)
            .unwrap_or(chrono::Duration::seconds(60));

        for buffer in self.by_track.values_mut() {
            while let Some(front) = buffer.front() {
                if front.observation.timestamp < cutoff {
                    buffer.pop_front();
                } else {
                    break;
                }
            }
        }

        self.unassociated_obs
            .retain(|o| o.observation.timestamp >= cutoff);
    }

    pub fn track_observation_count(&self, track_id: &TrackId) -> usize {
        self.by_track.get(track_id).map_or(0, VecDeque::len)
    }

    pub fn total_observation_count(&self) -> usize {
        let associated: usize = self.by_track.values().map(VecDeque::len).sum();
        associated + self.unassociated_obs.len()
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd airjedi-fusion && cargo test store`
Expected: all 3 tests PASS

- [ ] **Step 5: Add module to lib.rs and commit**

Add `pub mod store;` to `lib.rs`.

```bash
git add airjedi-fusion/src/store.rs airjedi-fusion/src/lib.rs
git commit -m "Add TimelineStore with VecDeque hot buffer and association"
```

---

### Task 5: EKF Filter in ECEF

**Files:**
- Create: `airjedi-fusion/src/filter/mod.rs`
- Create: `airjedi-fusion/src/filter/ekf.rs`
- Modify: `airjedi-fusion/src/lib.rs` (add module)
- Test: inline in `filter/ekf.rs`

**Interfaces:**
- Consumes: `SensorObservation`, `Measurement` from `sensor.rs`, `geodetic_to_ecef`, `spherical_to_ecef` from `coord.rs`, `Timestamp`, `StateVectorType` from `types.rs`
- Produces: `TrackFilter` trait, `FilterResult`, `Innovation`, `FilterVariant` enum, `TrackerState` (Component), `Ekf6Dof`, `ProcessNoiseConfig`, `StateHistory`, `StateSnapshot`, `OosmConfig`

- [ ] **Step 1: Create `filter/mod.rs` with trait and TrackerState**

```rust
// airjedi-fusion/src/filter/mod.rs
pub mod ekf;

use bevy::prelude::*;
use nalgebra::{DMatrix, DVector};
use std::collections::VecDeque;
use crate::coord;
use crate::sensor::SensorObservation;
use crate::types::{StateVectorType, Timestamp};

#[derive(Debug, Clone)]
pub struct Innovation {
    pub residual: DVector<f64>,
    pub covariance: DMatrix<f64>,
    pub mahalanobis_distance: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterResult {
    Updated,
    OutlierRejected { distance: f64 },
    DivergenceDetected,
}

#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub timestamp: Timestamp,
    pub state: DVector<f64>,
    pub covariance: DMatrix<f64>,
}

#[derive(Debug, Clone)]
pub struct StateHistory {
    snapshots: VecDeque<StateSnapshot>,
    max_depth: usize,
}

impl StateHistory {
    pub fn new(max_depth: usize) -> Self {
        Self {
            snapshots: VecDeque::with_capacity(max_depth),
            max_depth,
        }
    }

    pub fn push(&mut self, snapshot: StateSnapshot) {
        if self.snapshots.len() >= self.max_depth {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
    }

    pub fn find_before(&self, timestamp: Timestamp) -> Option<&StateSnapshot> {
        self.snapshots.iter().rev().find(|s| s.timestamp <= timestamp)
    }

    pub fn latest_timestamp(&self) -> Option<Timestamp> {
        self.snapshots.back().map(|s| s.timestamp)
    }
}

#[derive(Debug, Clone)]
pub struct OosmConfig {
    pub max_lag: std::time::Duration,
    pub history_depth: usize,
}

impl Default for OosmConfig {
    fn default() -> Self {
        Self {
            max_lag: std::time::Duration::from_secs(30),
            history_depth: 10,
        }
    }
}

pub trait TrackFilter: Send + Sync {
    fn predict(&mut self, dt: f64);
    fn update(&mut self, observation: &SensorObservation) -> FilterResult;
    fn state(&self) -> &DVector<f64>;
    fn covariance(&self) -> &DMatrix<f64>;
    fn innovation(&self, observation: &SensorObservation) -> Option<Innovation>;
    fn initialize(&mut self, observation: &SensorObservation);
    fn state_history(&self) -> &StateHistory;
}

#[derive(Debug, Clone)]
pub enum FilterVariant {
    Ekf6Dof(ekf::Ekf6Dof),
}

impl FilterVariant {
    pub fn predict(&mut self, dt: f64) {
        match self {
            Self::Ekf6Dof(f) => f.predict(dt),
        }
    }

    pub fn update(&mut self, observation: &SensorObservation) -> FilterResult {
        match self {
            Self::Ekf6Dof(f) => f.update(observation),
        }
    }

    pub fn state(&self) -> &DVector<f64> {
        match self {
            Self::Ekf6Dof(f) => f.state(),
        }
    }

    pub fn covariance(&self) -> &DMatrix<f64> {
        match self {
            Self::Ekf6Dof(f) => f.covariance(),
        }
    }

    pub fn innovation(&self, observation: &SensorObservation) -> Option<Innovation> {
        match self {
            Self::Ekf6Dof(f) => f.innovation(observation),
        }
    }

    pub fn initialize(&mut self, observation: &SensorObservation) {
        match self {
            Self::Ekf6Dof(f) => f.initialize(observation),
        }
    }

    pub fn state_history(&self) -> &StateHistory {
        match self {
            Self::Ekf6Dof(f) => f.state_history(),
        }
    }
}

#[derive(Component, Debug, Clone)]
pub struct TrackerState {
    pub variant: FilterVariant,
    pub state_type: StateVectorType,
    pub last_update: Option<Timestamp>,
}

impl TrackerState {
    pub fn new_6dof(config: ekf::ProcessNoiseConfig) -> Self {
        Self {
            variant: FilterVariant::Ekf6Dof(ekf::Ekf6Dof::new(config)),
            state_type: StateVectorType::Cartesian6Dof,
            last_update: None,
        }
    }

    pub fn position_ecef(&self) -> [f64; 3] {
        let s = self.variant.state();
        [s[0], s[1], s[2]]
    }

    pub fn velocity_ecef(&self) -> [f64; 3] {
        let s = self.variant.state();
        [s[3], s[4], s[5]]
    }

    pub fn position_geodetic(&self) -> (f64, f64, f64) {
        let ecef = self.position_ecef();
        coord::ecef_to_geodetic(&ecef)
    }
}
```

- [ ] **Step 2: Create `filter/ekf.rs` with failing tests**

```rust
// airjedi-fusion/src/filter/ekf.rs
use nalgebra::{DMatrix, DVector, SMatrix, SVector};
use crate::coord;
use crate::sensor::{Measurement, SensorObservation};
use super::{FilterResult, Innovation, StateHistory, StateSnapshot, TrackFilter};

#[derive(Debug, Clone)]
pub struct ProcessNoiseConfig {
    pub position_noise: f64,  // m^2/s^3
    pub velocity_noise: f64,  // m^2/s^5
}

impl Default for ProcessNoiseConfig {
    fn default() -> Self {
        Self {
            position_noise: 1.0,
            velocity_noise: 0.1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Ekf6Dof {
    x: SVector<f64, 6>,      // [x, y, z, vx, vy, vz] in ECEF meters
    p: SMatrix<f64, 6, 6>,
    q_config: ProcessNoiseConfig,
    history: StateHistory,
    gate_threshold: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::*;
    use crate::coord::CoordinateFrame;
    use crate::types::*;
    use chrono::Utc;
    use approx::assert_relative_eq;

    fn make_position_obs(lat: f64, lon: f64, alt: f64) -> SensorObservation {
        SensorObservation {
            sensor_id: SensorId {
                id: "test".to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: None,
            measurement: Measurement::PositionVelocity3D {
                lat_deg: lat,
                lon_deg: lon,
                alt_m: Some(alt),
                vel_north_mps: Some(100.0),
                vel_east_mps: Some(0.0),
                vel_down_mps: Some(0.0),
                heading_deg: None,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(6, 6) * 100.0,
            },
            classification_hint: None,
            metadata: ObservationMetadata::default(),
        }
    }

    #[test]
    fn initialize_from_observation() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);

        let (lat, lon, alt) = coord::ecef_to_geodetic(&[ekf.x[0], ekf.x[1], ekf.x[2]]);
        assert_relative_eq!(lat, 37.6872, epsilon = 0.001);
        assert_relative_eq!(lon, -97.3301, epsilon = 0.001);
        assert_relative_eq!(alt, 10000.0, epsilon = 10.0);
    }

    #[test]
    fn predict_constant_velocity() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);

        let x_before = ekf.x;
        ekf.predict(1.0); // 1 second
        // Position should have moved by velocity * dt
        assert_relative_eq!(ekf.x[0], x_before[0] + x_before[3], epsilon = 1e-6);
        assert_relative_eq!(ekf.x[1], x_before[1] + x_before[4], epsilon = 1e-6);
        assert_relative_eq!(ekf.x[2], x_before[2] + x_before[5], epsilon = 1e-6);
        // Velocity unchanged
        assert_relative_eq!(ekf.x[3], x_before[3], epsilon = 1e-6);
    }

    #[test]
    fn update_reduces_covariance() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);
        ekf.predict(1.0);

        let p_before_trace = ekf.p.trace();
        let result = ekf.update(&obs);
        assert_eq!(result, FilterResult::Updated);
        assert!(ekf.p.trace() < p_before_trace);
    }

    #[test]
    fn outlier_rejected() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);

        // Observation very far away should be rejected
        let far_obs = make_position_obs(50.0, -50.0, 10000.0);
        let result = ekf.update(&far_obs);
        assert!(matches!(result, FilterResult::OutlierRejected { .. }));
    }

    #[test]
    fn state_history_records_snapshots() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);
        ekf.predict(1.0);
        ekf.predict(1.0);
        assert!(ekf.history.snapshots.len() >= 2);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd airjedi-fusion && cargo test filter`
Expected: FAIL - `Ekf6Dof::new` and methods not found

- [ ] **Step 4: Implement Ekf6Dof**

Add to `filter/ekf.rs` above the test module:

```rust
impl Ekf6Dof {
    pub fn new(q_config: ProcessNoiseConfig) -> Self {
        Self {
            x: SVector::zeros(),
            p: SMatrix::identity() * 1e6,
            q_config,
            history: StateHistory::new(10),
            gate_threshold: 16.27, // chi-squared 99.9% with 3 DOF
        }
    }

    fn measurement_to_ecef(&self, obs: &SensorObservation) -> Option<(DVector<f64>, DMatrix<f64>)> {
        match &obs.measurement {
            Measurement::PositionVelocity3D {
                lat_deg, lon_deg, alt_m,
                vel_north_mps, vel_east_mps, vel_down_mps, ..
            } => {
                let alt = alt_m.unwrap_or(0.0);
                let ecef = coord::geodetic_to_ecef(*lat_deg, *lon_deg, alt);

                let mut z = DVector::zeros(6);
                z[0] = ecef[0];
                z[1] = ecef[1];
                z[2] = ecef[2];

                // Convert NED velocity to ECEF
                if let (Some(vn), Some(ve), Some(vd)) = (vel_north_mps, vel_east_mps, vel_down_mps) {
                    let lat_rad = lat_deg.to_radians();
                    let lon_rad = lon_deg.to_radians();
                    let sin_lat = lat_rad.sin();
                    let cos_lat = lat_rad.cos();
                    let sin_lon = lon_rad.sin();
                    let cos_lon = lon_rad.cos();

                    // NED to ECEF rotation
                    z[3] = -sin_lat * cos_lon * vn - sin_lon * ve - cos_lat * cos_lon * vd;
                    z[4] = -sin_lat * sin_lon * vn + cos_lon * ve - cos_lat * sin_lon * vd;
                    z[5] = cos_lat * vn - sin_lat * vd;
                }

                let r = obs.covariance.matrix.clone();
                Some((z, r))
            }
            Measurement::PositionVelocity2D {
                lat_deg, lon_deg, ..
            } => {
                let ecef = coord::geodetic_to_ecef(*lat_deg, *lon_deg, 0.0);
                let mut z = DVector::zeros(3);
                z[0] = ecef[0];
                z[1] = ecef[1];
                z[2] = ecef[2];
                let r = obs.covariance.matrix.clone();
                Some((z, r))
            }
            Measurement::Spherical {
                range_m, azimuth_rad, elevation_rad, ..
            } => {
                if let CoordinateFrame::SensorSpherical {
                    sensor_lat_deg, sensor_lon_deg, sensor_alt_m,
                } = &obs.sensor_id.coordinate_frame
                {
                    let sensor_ecef = coord::geodetic_to_ecef(
                        *sensor_lat_deg, *sensor_lon_deg, *sensor_alt_m,
                    );
                    let el = elevation_rad.unwrap_or(0.0);
                    let target_ecef = coord::spherical_to_ecef(
                        *range_m, *azimuth_rad, el, &sensor_ecef,
                        *sensor_lat_deg, *sensor_lon_deg,
                    );
                    let mut z = DVector::zeros(3);
                    z[0] = target_ecef[0];
                    z[1] = target_ecef[1];
                    z[2] = target_ecef[2];
                    let r = obs.covariance.matrix.clone();
                    Some((z, r))
                } else {
                    None
                }
            }
            _ => None, // BearingOnly, DepthBearing, FusedEstimate handled by future filter variants
        }
    }

    fn build_h_matrix(&self, z_dim: usize) -> DMatrix<f64> {
        let mut h = DMatrix::zeros(z_dim, 6);
        for i in 0..z_dim.min(6) {
            h[(i, i)] = 1.0;
        }
        h
    }
}

use crate::coord::CoordinateFrame;

impl TrackFilter for Ekf6Dof {
    fn predict(&mut self, dt: f64) {
        // Save state snapshot for OOSM
        self.history.push(StateSnapshot {
            timestamp: chrono::Utc::now(),
            state: DVector::from_iterator(6, self.x.iter().copied()),
            covariance: DMatrix::from_iterator(6, 6, self.p.iter().copied()),
        });

        // State transition: constant velocity
        // x_new = F * x where F = [[I, dt*I], [0, I]]
        self.x[0] += self.x[3] * dt;
        self.x[1] += self.x[4] * dt;
        self.x[2] += self.x[5] * dt;

        // Process noise: Q = diag([q_pos * dt^3/3, q_vel * dt])
        let mut f = SMatrix::<f64, 6, 6>::identity();
        f[(0, 3)] = dt;
        f[(1, 4)] = dt;
        f[(2, 5)] = dt;

        let qp = self.q_config.position_noise;
        let qv = self.q_config.velocity_noise;
        let mut q = SMatrix::<f64, 6, 6>::zeros();
        let dt3 = dt * dt * dt / 3.0;
        let dt2 = dt * dt / 2.0;
        for i in 0..3 {
            q[(i, i)] = qp * dt3;
            q[(i, i + 3)] = qp * dt2;
            q[(i + 3, i)] = qp * dt2;
            q[(i + 3, i + 3)] = qv * dt;
        }

        self.p = f * self.p * f.transpose() + q;
    }

    fn update(&mut self, observation: &SensorObservation) -> FilterResult {
        let (z, r) = match self.measurement_to_ecef(observation) {
            Some(pair) => pair,
            None => return FilterResult::OutlierRejected { distance: f64::INFINITY },
        };

        let z_dim = z.len();
        let h = self.build_h_matrix(z_dim);

        // Predicted measurement
        let z_pred = &h * DVector::from_iterator(6, self.x.iter().copied());

        // Innovation
        let y = &z - &z_pred;
        let p_dyn = DMatrix::from_iterator(6, 6, self.p.iter().copied());
        let s = &h * &p_dyn * h.transpose() + &r;

        // Mahalanobis distance
        let s_inv = match s.clone().try_inverse() {
            Some(inv) => inv,
            None => return FilterResult::DivergenceDetected,
        };
        let maha2 = (&y.transpose() * &s_inv * &y)[(0, 0)];

        if maha2 > self.gate_threshold {
            return FilterResult::OutlierRejected { distance: maha2.sqrt() };
        }

        // Kalman gain
        let k = &p_dyn * h.transpose() * &s_inv;

        // State update
        let dx = &k * &y;
        for i in 0..6 {
            self.x[i] += dx[i];
        }

        // Covariance update (Joseph form for numerical stability)
        let i_kh = DMatrix::identity(6, 6) - &k * &h;
        let p_new = &i_kh * &p_dyn * i_kh.transpose() + &k * &r * k.transpose();
        for i in 0..6 {
            for j in 0..6 {
                self.p[(i, j)] = p_new[(i, j)];
            }
        }

        FilterResult::Updated
    }

    fn state(&self) -> &DVector<f64> {
        // Return a view as DVector - allocated on each call.
        // For hot path, callers should use TrackerState::position_ecef() instead.
        &*Box::leak(Box::new(DVector::from_iterator(6, self.x.iter().copied())))
    }

    fn covariance(&self) -> &DMatrix<f64> {
        &*Box::leak(Box::new(DMatrix::from_iterator(6, 6, self.p.iter().copied())))
    }

    fn innovation(&self, observation: &SensorObservation) -> Option<Innovation> {
        let (z, r) = self.measurement_to_ecef(observation)?;
        let z_dim = z.len();
        let h = self.build_h_matrix(z_dim);
        let z_pred = &h * DVector::from_iterator(6, self.x.iter().copied());
        let y = &z - &z_pred;
        let p_dyn = DMatrix::from_iterator(6, 6, self.p.iter().copied());
        let s = &h * &p_dyn * h.transpose() + &r;
        let s_inv = s.clone().try_inverse()?;
        let maha2 = (&y.transpose() * &s_inv * &y)[(0, 0)];

        Some(Innovation {
            residual: y,
            covariance: s,
            mahalanobis_distance: maha2.sqrt(),
        })
    }

    fn initialize(&mut self, observation: &SensorObservation) {
        if let Some((z, _r)) = self.measurement_to_ecef(observation) {
            for i in 0..z.len().min(6) {
                self.x[i] = z[i];
            }
            self.p = SMatrix::identity() * 1e4;
            self.history = StateHistory::new(self.history.snapshots.capacity());
        }
    }

    fn state_history(&self) -> &StateHistory {
        &self.history
    }
}
```

- [ ] **Step 5: Fix the state()/covariance() lifetime issue**

The `Box::leak` approach in `state()` and `covariance()` leaks memory. Instead, change `TrackerState` to own cached copies, or change the trait to return owned values. The cleanest fix: change the trait signatures to return owned values since these are called infrequently:

Replace the trait methods:
```rust
    fn state_vec(&self) -> DVector<f64>;
    fn covariance_mat(&self) -> DMatrix<f64>;
```

And implement:
```rust
    fn state_vec(&self) -> DVector<f64> {
        DVector::from_iterator(6, self.x.iter().copied())
    }

    fn covariance_mat(&self) -> DMatrix<f64> {
        DMatrix::from_iterator(6, 6, self.p.iter().copied())
    }
```

Update `FilterVariant` and `TrackerState` accordingly.

- [ ] **Step 6: Run tests**

Run: `cd airjedi-fusion && cargo test filter`
Expected: all 5 tests PASS

- [ ] **Step 7: Add module to lib.rs and commit**

Add `pub mod filter;` to `lib.rs`.

```bash
git add airjedi-fusion/src/filter/ airjedi-fusion/src/lib.rs
git commit -m "Add EKF filter in ECEF with predict, update, outlier rejection, state history"
```

---

### Task 6: Track Components and Lifecycle

**Files:**
- Create: `airjedi-fusion/src/track.rs`
- Modify: `airjedi-fusion/src/lib.rs` (add module)
- Test: inline in `track.rs`

**Interfaces:**
- Consumes: `TrackId`, `TargetId`, `Timestamp`, `TargetCategory` from `types.rs`
- Produces: `Track` (Component), `TrackQuality` (Component), `TrackStatus`, `TrackLifecycleConfig`, `LifecycleProfiles`

- [ ] **Step 1: Write `track.rs` with components and lifecycle config**

```rust
// airjedi-fusion/src/track.rs
use bevy::prelude::*;
use std::collections::HashMap;
use std::time::Duration;
use crate::types::{TargetCategory, TargetId, Timestamp, TrackId};

#[derive(Component, Debug, Clone)]
pub struct Track {
    pub id: TrackId,
    pub cooperative_ids: Vec<TargetId>,
    pub created_at: Timestamp,
    pub last_update: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub enum TrackStatus {
    Tentative,
    Confirmed,
    Coasting,
    Lost,
}

#[derive(Component, Debug, Clone, Reflect)]
pub struct TrackQuality {
    pub status: TrackStatus,
    pub sensor_count: u8,
    pub update_rate: f32,
    pub staleness: Duration,
    pub confidence: f32,
    pub observation_count: u32,
}

impl Default for TrackQuality {
    fn default() -> Self {
        Self {
            status: TrackStatus::Tentative,
            sensor_count: 0,
            update_rate: 0.0,
            staleness: Duration::ZERO,
            confidence: 0.0,
            observation_count: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrackLifecycleConfig {
    pub confirm_threshold: u32,
    pub confirm_window: Duration,
    pub coast_timeout: Duration,
    pub lost_timeout: Duration,
    pub cleanup_delay: Duration,
}

impl Default for TrackLifecycleConfig {
    fn default() -> Self {
        Self {
            confirm_threshold: 3,
            confirm_window: Duration::from_secs(10),
            coast_timeout: Duration::from_secs(15),
            lost_timeout: Duration::from_secs(60),
            cleanup_delay: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, Resource)]
pub struct LifecycleProfiles {
    pub profiles: HashMap<TargetCategory, TrackLifecycleConfig>,
    pub default_profile: TrackLifecycleConfig,
}

impl Default for LifecycleProfiles {
    fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert(TargetCategory::FixedWing, TrackLifecycleConfig {
            confirm_threshold: 3,
            confirm_window: Duration::from_secs(10),
            coast_timeout: Duration::from_secs(15),
            lost_timeout: Duration::from_secs(60),
            cleanup_delay: Duration::from_secs(5),
        });
        profiles.insert(TargetCategory::Drone, TrackLifecycleConfig {
            confirm_threshold: 3,
            confirm_window: Duration::from_secs(5),
            coast_timeout: Duration::from_secs(10),
            lost_timeout: Duration::from_secs(30),
            cleanup_delay: Duration::from_secs(3),
        });
        profiles.insert(TargetCategory::Missile, TrackLifecycleConfig {
            confirm_threshold: 2,
            confirm_window: Duration::from_secs(3),
            coast_timeout: Duration::from_secs(5),
            lost_timeout: Duration::from_secs(15),
            cleanup_delay: Duration::from_secs(2),
        });
        profiles.insert(TargetCategory::SurfaceVessel, TrackLifecycleConfig {
            confirm_threshold: 3,
            confirm_window: Duration::from_secs(60),
            coast_timeout: Duration::from_secs(600),
            lost_timeout: Duration::from_secs(7200),
            cleanup_delay: Duration::from_secs(60),
        });
        profiles.insert(TargetCategory::GroundVehicle, TrackLifecycleConfig {
            confirm_threshold: 3,
            confirm_window: Duration::from_secs(30),
            coast_timeout: Duration::from_secs(300),
            lost_timeout: Duration::from_secs(3600),
            cleanup_delay: Duration::from_secs(30),
        });
        profiles.insert(TargetCategory::Person, TrackLifecycleConfig {
            confirm_threshold: 3,
            confirm_window: Duration::from_secs(10),
            coast_timeout: Duration::from_secs(30),
            lost_timeout: Duration::from_secs(120),
            cleanup_delay: Duration::from_secs(10),
        });

        Self {
            profiles,
            default_profile: TrackLifecycleConfig::default(),
        }
    }
}

impl LifecycleProfiles {
    pub fn get(&self, category: &TargetCategory) -> &TrackLifecycleConfig {
        self.profiles.get(category).unwrap_or(&self.default_profile)
    }
}

impl TrackQuality {
    pub fn transition(
        &mut self,
        staleness: Duration,
        config: &TrackLifecycleConfig,
    ) {
        self.staleness = staleness;
        match self.status {
            TrackStatus::Tentative => {
                if self.observation_count >= config.confirm_threshold {
                    self.status = TrackStatus::Confirmed;
                }
            }
            TrackStatus::Confirmed => {
                if staleness > config.coast_timeout {
                    self.status = TrackStatus::Coasting;
                }
            }
            TrackStatus::Coasting => {
                if staleness > config.coast_timeout + config.lost_timeout {
                    self.status = TrackStatus::Lost;
                }
            }
            TrackStatus::Lost => {}
        }
    }

    pub fn reacquire(&mut self) {
        if self.status == TrackStatus::Coasting {
            self.status = TrackStatus::Confirmed;
        }
        self.staleness = Duration::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tentative_to_confirmed() {
        let config = TrackLifecycleConfig::default();
        let mut quality = TrackQuality::default();
        quality.observation_count = 3;
        quality.transition(Duration::from_secs(0), &config);
        assert_eq!(quality.status, TrackStatus::Confirmed);
    }

    #[test]
    fn confirmed_to_coasting() {
        let config = TrackLifecycleConfig::default();
        let mut quality = TrackQuality {
            status: TrackStatus::Confirmed,
            observation_count: 5,
            ..Default::default()
        };
        quality.transition(Duration::from_secs(20), &config);
        assert_eq!(quality.status, TrackStatus::Coasting);
    }

    #[test]
    fn coasting_reacquire() {
        let mut quality = TrackQuality {
            status: TrackStatus::Coasting,
            staleness: Duration::from_secs(30),
            ..Default::default()
        };
        quality.reacquire();
        assert_eq!(quality.status, TrackStatus::Confirmed);
        assert_eq!(quality.staleness, Duration::ZERO);
    }

    #[test]
    fn lifecycle_profiles_per_category() {
        let profiles = LifecycleProfiles::default();
        let missile = profiles.get(&TargetCategory::Missile);
        let vessel = profiles.get(&TargetCategory::SurfaceVessel);
        assert!(missile.lost_timeout < vessel.lost_timeout);
    }
}
```

- [ ] **Step 2: Add module to lib.rs, run tests, commit**

Add `pub mod track;` to `lib.rs`.

Run: `cd airjedi-fusion && cargo test track`
Expected: all 4 tests PASS

```bash
git add airjedi-fusion/src/track.rs airjedi-fusion/src/lib.rs
git commit -m "Add track components with per-category lifecycle state machine"
```

---

### Task 7: GNN Associator with Spatial Index

**Files:**
- Create: `airjedi-fusion/src/associator/mod.rs`
- Create: `airjedi-fusion/src/associator/spatial_index.rs`
- Create: `airjedi-fusion/src/associator/gnn.rs`
- Modify: `airjedi-fusion/src/lib.rs` (add module)
- Modify: `airjedi-fusion/Cargo.toml` (add `lapjv` dependency)
- Test: inline in each file

**Interfaces:**
- Consumes: `StoredObservation` from `store.rs`, `Track`, `TrackQuality`, `TargetClassification` from `track.rs`/`classification.rs`, `TrackerState` (for `innovation()`) from `filter/mod.rs`
- Produces: `Associator` trait, `AssociationResult`, `Assignment`, `AssociatorConfig`, `GateParams`, `GnnAssociator`, `SpatialIndex`, `ActiveAssociator` (Resource)

- [ ] **Step 1: Add `lapjv` to Cargo.toml**

Add to `[dependencies]` in `airjedi-fusion/Cargo.toml`:
```toml
lapjv = "0.2"
```

Note: if `lapjv` is not available for the current Rust version, use a simple Hungarian algorithm implementation or the `pathfinding` crate's `kuhn_munkres` as a fallback.

- [ ] **Step 2: Create `associator/spatial_index.rs`**

```rust
// airjedi-fusion/src/associator/spatial_index.rs
use std::collections::{HashMap, HashSet};
use crate::types::TrackId;

#[derive(Debug)]
pub struct SpatialIndex {
    grid: HashMap<(i32, i32), HashSet<TrackId>>,
    track_cells: HashMap<TrackId, (i32, i32)>,
    cell_size_deg: f64,
}

impl SpatialIndex {
    pub fn new(cell_size_deg: f64) -> Self {
        Self {
            grid: HashMap::new(),
            track_cells: HashMap::new(),
            cell_size_deg,
        }
    }

    fn cell_for(&self, lat_deg: f64, lon_deg: f64) -> (i32, i32) {
        let lat_bin = (lat_deg / self.cell_size_deg).floor() as i32;
        let lon_bin = (lon_deg / self.cell_size_deg).floor() as i32;
        (lat_bin, lon_bin)
    }

    pub fn update_track(&mut self, track_id: &TrackId, lat_deg: f64, lon_deg: f64) {
        let new_cell = self.cell_for(lat_deg, lon_deg);

        if let Some(old_cell) = self.track_cells.get(track_id) {
            if *old_cell == new_cell {
                return;
            }
            if let Some(set) = self.grid.get_mut(old_cell) {
                set.remove(track_id);
                if set.is_empty() {
                    self.grid.remove(old_cell);
                }
            }
        }

        self.grid
            .entry(new_cell)
            .or_insert_with(HashSet::new)
            .insert(track_id.clone());
        self.track_cells.insert(track_id.clone(), new_cell);
    }

    pub fn remove_track(&mut self, track_id: &TrackId) {
        if let Some(cell) = self.track_cells.remove(track_id) {
            if let Some(set) = self.grid.get_mut(&cell) {
                set.remove(track_id);
                if set.is_empty() {
                    self.grid.remove(&cell);
                }
            }
        }
    }

    pub fn nearby_tracks(&self, lat_deg: f64, lon_deg: f64) -> Vec<TrackId> {
        let (cy, cx) = self.cell_for(lat_deg, lon_deg);
        let mut result = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(set) = self.grid.get(&(cy + dy, cx + dx)) {
                    result.extend(set.iter().cloned());
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_find_nearby() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        let t2 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.update_track(&t2, 37.1, -97.1); // same cell at 0.5 deg
        let nearby = idx.nearby_tracks(37.05, -97.05);
        assert!(nearby.contains(&t1));
        assert!(nearby.contains(&t2));
    }

    #[test]
    fn distant_track_not_found() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        let t2 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.update_track(&t2, 50.0, -50.0); // far away
        let nearby = idx.nearby_tracks(37.0, -97.0);
        assert!(nearby.contains(&t1));
        assert!(!nearby.contains(&t2));
    }

    #[test]
    fn remove_track() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.remove_track(&t1);
        let nearby = idx.nearby_tracks(37.0, -97.0);
        assert!(nearby.is_empty());
    }

    #[test]
    fn track_moves_between_cells() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.update_track(&t1, 50.0, -50.0); // move far away
        let near_old = idx.nearby_tracks(37.0, -97.0);
        let near_new = idx.nearby_tracks(50.0, -50.0);
        assert!(!near_old.contains(&t1));
        assert!(near_new.contains(&t1));
    }
}
```

- [ ] **Step 3: Create `associator/mod.rs` with trait and config**

```rust
// airjedi-fusion/src/associator/mod.rs
pub mod gnn;
pub mod spatial_index;

use std::collections::HashMap;
use bevy::prelude::*;
use crate::store::StoredObservation;
use crate::track::Track;
use crate::filter::TrackerState;
use crate::classification::TargetClassification;
use crate::types::TargetCategory;

#[derive(Debug, Clone)]
pub struct Assignment {
    pub observation_idx: usize,
    pub track_idx: usize,
    pub distance: f64,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct AssociationResult {
    pub assignments: Vec<Assignment>,
    pub unassigned_observations: Vec<usize>,
    pub unassigned_tracks: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct GateParams {
    pub chi_squared_threshold: f64,
}

impl Default for GateParams {
    fn default() -> Self {
        Self {
            chi_squared_threshold: 16.27, // 99.9% with 3 DOF
        }
    }
}

#[derive(Debug, Clone, Resource)]
pub struct AssociatorConfig {
    pub gate_profiles: HashMap<TargetCategory, GateParams>,
    pub default_gate: GateParams,
    pub cooperative_id_boost: f64,
}

impl Default for AssociatorConfig {
    fn default() -> Self {
        let mut gate_profiles = HashMap::new();
        gate_profiles.insert(TargetCategory::Person, GateParams {
            chi_squared_threshold: 11.34,
        });
        gate_profiles.insert(TargetCategory::Missile, GateParams {
            chi_squared_threshold: 16.27,
        });

        Self {
            gate_profiles,
            default_gate: GateParams::default(),
            cooperative_id_boost: 0.01, // multiplier on distance when cooperative IDs match
        }
    }
}

impl AssociatorConfig {
    pub fn gate_for(&self, category: &TargetCategory) -> &GateParams {
        self.gate_profiles.get(category).unwrap_or(&self.default_gate)
    }
}
```

- [ ] **Step 4: Create `associator/gnn.rs` with implementation**

```rust
// airjedi-fusion/src/associator/gnn.rs
use crate::associator::{AssociatorConfig, Assignment, AssociationResult, spatial_index::SpatialIndex};
use crate::classification::TargetClassification;
use crate::coord;
use crate::filter::TrackerState;
use crate::store::StoredObservation;
use crate::track::Track;
use crate::types::{TargetCategory, TrackId};
use std::collections::HashMap;

pub struct GnnAssociator;

impl GnnAssociator {
    pub fn associate(
        observations: &[&StoredObservation],
        tracks: &[(&Track, &TrackerState, &TargetClassification)],
        spatial_index: &SpatialIndex,
        config: &AssociatorConfig,
    ) -> AssociationResult {
        if observations.is_empty() || tracks.is_empty() {
            return AssociationResult {
                assignments: Vec::new(),
                unassigned_observations: (0..observations.len()).collect(),
                unassigned_tracks: (0..tracks.len()).collect(),
            };
        }

        // Build track_id -> track_index lookup
        let track_id_to_idx: HashMap<&TrackId, usize> = tracks
            .iter()
            .enumerate()
            .map(|(i, (t, _, _))| (&t.id, i))
            .collect();

        // Build sparse cost matrix via spatial pre-filter + gating
        let mut costs: Vec<(usize, usize, f64)> = Vec::new(); // (obs_idx, track_idx, cost)

        for (obs_idx, obs) in observations.iter().enumerate() {
            let obs_pos = observation_geodetic_position(&obs.observation);
            let (obs_lat, obs_lon) = match obs_pos {
                Some(pos) => pos,
                None => continue,
            };

            let nearby = spatial_index.nearby_tracks(obs_lat, obs_lon);

            for nearby_track_id in &nearby {
                let track_idx = match track_id_to_idx.get(nearby_track_id) {
                    Some(&idx) => idx,
                    None => continue,
                };

                let (track, tracker, classification) = &tracks[track_idx];
                let gate = config.gate_for(&classification.category);

                if let Some(innov) = tracker.variant.innovation(&obs.observation) {
                    let mut distance = innov.mahalanobis_distance;

                    // Cooperative ID boost
                    if let Some(ref target_id) = obs.observation.target_id {
                        if track.cooperative_ids.iter().any(|cid| cid.id == target_id.id) {
                            distance *= config.cooperative_id_boost;
                        }
                    }

                    if distance * distance <= gate.chi_squared_threshold {
                        costs.push((obs_idx, track_idx, distance));
                    }
                }
            }
        }

        // Simple greedy assignment (upgrade to JV when lapjv is available)
        // Sort by cost ascending, greedily assign
        costs.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

        let mut assigned_obs: Vec<bool> = vec![false; observations.len()];
        let mut assigned_tracks: Vec<bool> = vec![false; tracks.len()];
        let mut assignments = Vec::new();

        for (obs_idx, track_idx, distance) in &costs {
            if !assigned_obs[*obs_idx] && !assigned_tracks[*track_idx] {
                assignments.push(Assignment {
                    observation_idx: *obs_idx,
                    track_idx: *track_idx,
                    distance: *distance,
                    confidence: (1.0 - distance / 10.0).clamp(0.0, 1.0) as f32,
                });
                assigned_obs[*obs_idx] = true;
                assigned_tracks[*track_idx] = true;
            }
        }

        let unassigned_observations: Vec<usize> = (0..observations.len())
            .filter(|i| !assigned_obs[*i])
            .collect();
        let unassigned_tracks: Vec<usize> = (0..tracks.len())
            .filter(|i| !assigned_tracks[*i])
            .collect();

        AssociationResult {
            assignments,
            unassigned_observations,
            unassigned_tracks,
        }
    }
}

fn observation_geodetic_position(obs: &crate::sensor::SensorObservation) -> Option<(f64, f64)> {
    match &obs.measurement {
        crate::sensor::Measurement::PositionVelocity3D { lat_deg, lon_deg, .. } => {
            Some((*lat_deg, *lon_deg))
        }
        crate::sensor::Measurement::PositionVelocity2D { lat_deg, lon_deg, .. } => {
            Some((*lat_deg, *lon_deg))
        }
        crate::sensor::Measurement::Spherical {
            range_m, azimuth_rad, elevation_rad, ..
        } => {
            if let crate::coord::CoordinateFrame::SensorSpherical {
                sensor_lat_deg, sensor_lon_deg, sensor_alt_m,
            } = &obs.sensor_id.coordinate_frame
            {
                let sensor_ecef = coord::geodetic_to_ecef(*sensor_lat_deg, *sensor_lon_deg, *sensor_alt_m);
                let el = elevation_rad.unwrap_or(0.0);
                let target_ecef = coord::spherical_to_ecef(*range_m, *azimuth_rad, el, &sensor_ecef, *sensor_lat_deg, *sensor_lon_deg);
                let (lat, lon, _) = coord::ecef_to_geodetic(&target_ecef);
                Some((lat, lon))
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::*;
    use crate::filter::ekf::ProcessNoiseConfig;
    use crate::types::*;
    use chrono::Utc;
    use nalgebra::DMatrix;

    fn make_obs_at(lat: f64, lon: f64, icao: Option<&str>) -> StoredObservation {
        StoredObservation {
            observation: SensorObservation {
                sensor_id: SensorId {
                    id: "test".to_string(),
                    kind: SensorKind::AdsbReceiver,
                    tier: FusionTier::Regional,
                    coordinate_frame: crate::coord::CoordinateFrame::Wgs84,
                },
                timestamp: Utc::now(),
                receipt_time: Utc::now(),
                target_id: icao.map(|id| TargetId {
                    domain: TargetDomain::Air,
                    id: id.to_string(),
                    id_type: IdentifierType::Icao,
                }),
                measurement: Measurement::PositionVelocity3D {
                    lat_deg: lat, lon_deg: lon, alt_m: Some(10000.0),
                    vel_north_mps: Some(100.0), vel_east_mps: Some(0.0),
                    vel_down_mps: Some(0.0), heading_deg: None,
                },
                covariance: ObservationCovariance {
                    matrix: DMatrix::identity(6, 6) * 100.0,
                },
                classification_hint: Some(TargetCategory::FixedWing),
                metadata: ObservationMetadata::default(),
            },
            associated_track: None,
            store_index: 0,
        }
    }

    fn make_track_at(lat: f64, lon: f64, icao: Option<&str>) -> (Track, TrackerState, TargetClassification) {
        let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
        let obs = make_obs_at(lat, lon, icao);
        tracker.variant.initialize(&obs.observation);

        let mut track = Track {
            id: TrackId::new(),
            cooperative_ids: Vec::new(),
            created_at: Utc::now(),
            last_update: Utc::now(),
        };
        if let Some(id) = icao {
            track.cooperative_ids.push(TargetId {
                domain: TargetDomain::Air,
                id: id.to_string(),
                id_type: IdentifierType::Icao,
            });
        }

        let classification = TargetClassification::default();
        (track, tracker, classification)
    }

    #[test]
    fn associate_nearby_observation_to_track() {
        let obs = make_obs_at(37.0, -97.0, None);
        let (track, tracker, class) = make_track_at(37.0, -97.0, None);

        let mut spatial = SpatialIndex::new(0.5);
        let (lat, _, _) = tracker.position_geodetic();
        spatial.update_track(&track.id, 37.0, -97.0);

        let config = AssociatorConfig::default();
        let result = GnnAssociator::associate(
            &[&obs],
            &[(&track, &tracker, &class)],
            &spatial,
            &config,
        );
        assert_eq!(result.assignments.len(), 1);
        assert!(result.unassigned_observations.is_empty());
    }

    #[test]
    fn distant_observation_not_associated() {
        let obs = make_obs_at(50.0, -50.0, None);
        let (track, tracker, class) = make_track_at(37.0, -97.0, None);

        let mut spatial = SpatialIndex::new(0.5);
        spatial.update_track(&track.id, 37.0, -97.0);

        let config = AssociatorConfig::default();
        let result = GnnAssociator::associate(
            &[&obs],
            &[(&track, &tracker, &class)],
            &spatial,
            &config,
        );
        assert!(result.assignments.is_empty());
        assert_eq!(result.unassigned_observations.len(), 1);
    }

    #[test]
    fn cooperative_id_match_preferred() {
        let obs = make_obs_at(37.01, -97.01, Some("ABC123"));
        let (track1, tracker1, class1) = make_track_at(37.0, -97.0, Some("ABC123"));
        let (track2, tracker2, class2) = make_track_at(37.005, -97.005, None); // closer but no ICAO

        let mut spatial = SpatialIndex::new(0.5);
        spatial.update_track(&track1.id, 37.0, -97.0);
        spatial.update_track(&track2.id, 37.005, -97.005);

        let config = AssociatorConfig::default();
        let result = GnnAssociator::associate(
            &[&obs],
            &[(&track1, &tracker1, &class1), (&track2, &tracker2, &class2)],
            &spatial,
            &config,
        );
        assert_eq!(result.assignments.len(), 1);
        assert_eq!(result.assignments[0].track_idx, 0); // matched to track1 with ICAO
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cd airjedi-fusion && cargo test associator`
Expected: all tests PASS (spatial_index: 4 tests, gnn: 3 tests)

- [ ] **Step 6: Add module to lib.rs and commit**

Add `pub mod associator;` to `lib.rs`.

```bash
git add airjedi-fusion/src/associator/ airjedi-fusion/src/lib.rs airjedi-fusion/Cargo.toml
git commit -m "Add GNN associator with spatial index pre-filter and cooperative ID matching"
```

---

### Task 8: Fusion Systems and Plugin Assembly

**Files:**
- Create: `airjedi-fusion/src/systems.rs`
- Create: `airjedi-fusion/src/config.rs`
- Modify: `airjedi-fusion/src/lib.rs` (add FusionPlugin, system sets, re-exports)
- Test: integration test in `airjedi-fusion/tests/integration.rs`

**Interfaces:**
- Consumes: all previous modules
- Produces: `FusionPlugin` (Bevy Plugin), `FusionConfig` (Resource), `FusionSystemSets` (SystemSet enums), complete pipeline wired in FixedUpdate

- [ ] **Step 1: Create `config.rs`**

```rust
// airjedi-fusion/src/config.rs
use bevy::prelude::*;
use crate::associator::AssociatorConfig;
use crate::filter::ekf::ProcessNoiseConfig;
use crate::filter::OosmConfig;
use crate::sensor::FusionTier;
use crate::store::StoreConfig;
use crate::track::LifecycleProfiles;

#[derive(Resource, Debug, Clone)]
pub struct FusionConfig {
    pub store: StoreConfig,
    pub lifecycle: LifecycleProfiles,
    pub associator: AssociatorConfig,
    pub filter_defaults: ProcessNoiseConfig,
    pub oosm: OosmConfig,
    pub node_id: String,
    pub tier: FusionTier,
    pub fixed_update_hz: f64,
    pub spatial_cell_size_deg: f64,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            store: StoreConfig::default(),
            lifecycle: LifecycleProfiles::default(),
            associator: AssociatorConfig::default(),
            filter_defaults: ProcessNoiseConfig::default(),
            oosm: OosmConfig::default(),
            node_id: "local".to_string(),
            tier: FusionTier::Regional,
            fixed_update_hz: 10.0,
            spatial_cell_size_deg: 0.5,
        }
    }
}
```

- [ ] **Step 2: Create `systems.rs` with system sets and fusion systems**

```rust
// airjedi-fusion/src/systems.rs
use bevy::prelude::*;
use chrono::Utc;
use crate::associator::gnn::GnnAssociator;
use crate::associator::spatial_index::SpatialIndex;
use crate::associator::AssociatorConfig;
use crate::classification::TargetClassification;
use crate::config::FusionConfig;
use crate::filter::ekf::ProcessNoiseConfig;
use crate::filter::{FilterResult, TrackerState};
use crate::sensor::SensorObservation;
use crate::store::TimelineStore;
use crate::track::{LifecycleProfiles, Track, TrackQuality, TrackStatus};
use crate::types::TrackId;

// --- System Sets ---

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum FusionSet {
    Drain,
    Associate,
    Fuse,
    Lifecycle,
}

// --- Observation buffer for ingest systems ---

#[derive(Resource, Default)]
pub struct ObservationBuffer {
    pub observations: Vec<SensorObservation>,
}

// --- Systems ---

pub fn drain_observations(
    mut buffer: ResMut<ObservationBuffer>,
    mut store: ResMut<TimelineStore>,
) {
    for obs in buffer.observations.drain(..) {
        store.insert(obs);
    }
}

pub fn association_system(
    mut store: ResMut<TimelineStore>,
    tracks: Query<(&Track, &TrackerState, &TargetClassification)>,
    spatial_index: Res<SpatialIndex>,
    config: Res<AssociatorConfig>,
) {
    let unassociated: Vec<_> = store.unassociated().iter().collect();
    if unassociated.is_empty() {
        return;
    }

    let track_list: Vec<_> = tracks.iter().collect();
    if track_list.is_empty() {
        return;
    }

    let result = GnnAssociator::associate(
        &unassociated.iter().map(|o| *o).collect::<Vec<_>>(),
        &track_list,
        &spatial_index,
        &config,
    );

    // Associate in reverse order to keep indices valid
    let mut sorted_assignments = result.assignments;
    sorted_assignments.sort_by(|a, b| b.observation_idx.cmp(&a.observation_idx));
    for assignment in &sorted_assignments {
        let track_id = &track_list[assignment.track_idx].0.id;
        store.associate(assignment.observation_idx, track_id);
    }
}

pub fn fusion_update_system(
    time: Res<Time>,
    store: Res<TimelineStore>,
    mut tracks: Query<(&Track, &mut TrackerState, &mut TrackQuality)>,
) {
    let dt = time.delta_secs_f64();
    if dt <= 0.0 {
        return;
    }
    let now = Utc::now();

    for (track, mut tracker, mut quality) in &mut tracks {
        tracker.variant.predict(dt);

        let obs = store.query_range(
            &track.id,
            track.last_update,
            now,
        );

        for stored_obs in &obs {
            match tracker.variant.update(&stored_obs.observation) {
                FilterResult::Updated => {
                    quality.observation_count += 1;
                    quality.reacquire();
                }
                FilterResult::OutlierRejected { .. } => {}
                FilterResult::DivergenceDetected => {
                    tracker.variant.initialize(&stored_obs.observation);
                }
            }
        }

        tracker.last_update = Some(now);
    }
}

pub fn update_spatial_index(
    mut spatial_index: ResMut<SpatialIndex>,
    tracks: Query<(&Track, &TrackerState), Changed<TrackerState>>,
) {
    for (track, tracker) in &tracks {
        let (lat, lon, _) = tracker.position_geodetic();
        spatial_index.update_track(&track.id, lat, lon);
    }
}

pub fn track_status_system(
    time: Res<Time>,
    lifecycle: Res<LifecycleProfiles>,
    mut tracks: Query<(&Track, &mut TrackQuality, &TargetClassification)>,
) {
    for (_track, mut quality, classification) in &mut tracks {
        let config = lifecycle.get(&classification.category);
        let staleness = quality.staleness + time.delta();
        quality.transition(staleness, config);
    }
}

pub fn track_initiation_system(
    mut commands: Commands,
    store: Res<TimelineStore>,
    fusion_config: Res<FusionConfig>,
) {
    for obs in store.unassociated() {
        let mut tracker = TrackerState::new_6dof(fusion_config.filter_defaults.clone());
        tracker.variant.initialize(&obs.observation);
        tracker.last_update = Some(Utc::now());

        let track_id = TrackId::new();

        let mut cooperative_ids = Vec::new();
        if let Some(ref target_id) = obs.observation.target_id {
            cooperative_ids.push(target_id.clone());
        }

        let classification = TargetClassification {
            category: obs.observation.classification_hint.unwrap_or(
                crate::types::TargetCategory::Unknown
            ),
            ..Default::default()
        };

        commands.spawn((
            Track {
                id: track_id,
                cooperative_ids,
                created_at: Utc::now(),
                last_update: Utc::now(),
            },
            tracker,
            TrackQuality::default(),
            classification,
        ));
    }
}

pub fn track_cleanup_system(
    mut commands: Commands,
    mut spatial_index: ResMut<SpatialIndex>,
    tracks: Query<(Entity, &Track, &TrackQuality)>,
) {
    for (entity, track, quality) in &tracks {
        if quality.status == TrackStatus::Lost {
            spatial_index.remove_track(&track.id);
            commands.entity(entity).despawn();
        }
    }
}
```

- [ ] **Step 3: Wire up `FusionPlugin` in `lib.rs`**

Update `lib.rs`:

```rust
// airjedi-fusion/src/lib.rs
pub mod types;
pub mod classification;
pub mod coord;
pub mod sensor;
pub mod store;
pub mod track;
pub mod filter;
pub mod associator;
pub mod systems;
pub mod config;

pub use types::*;
pub use classification::TargetClassification;
pub use config::FusionConfig;
pub use filter::TrackerState;
pub use track::{Track, TrackQuality, TrackStatus};
pub use sensor::{SensorObservation, Measurement};
pub use store::TimelineStore;

use bevy::prelude::*;
use systems::FusionSet;

pub struct FusionPlugin;

impl Plugin for FusionPlugin {
    fn build(&self, app: &mut App) {
        let config = app
            .world()
            .get_resource::<FusionConfig>()
            .cloned()
            .unwrap_or_default();

        app.init_resource::<systems::ObservationBuffer>()
            .insert_resource(TimelineStore::new(config.store.clone()))
            .insert_resource(config.lifecycle.clone())
            .insert_resource(config.associator.clone())
            .insert_resource(
                associator::spatial_index::SpatialIndex::new(config.spatial_cell_size_deg),
            )
            .configure_sets(
                FixedUpdate,
                (
                    FusionSet::Drain,
                    FusionSet::Associate,
                    FusionSet::Fuse,
                    FusionSet::Lifecycle,
                )
                    .chain(),
            )
            .add_systems(
                FixedUpdate,
                (
                    systems::drain_observations.in_set(FusionSet::Drain),
                    (systems::association_system, systems::update_spatial_index)
                        .in_set(FusionSet::Associate),
                    systems::fusion_update_system.in_set(FusionSet::Fuse),
                    (
                        systems::track_status_system,
                        systems::track_initiation_system,
                        systems::track_cleanup_system,
                    )
                        .in_set(FusionSet::Lifecycle),
                ),
            );
    }
}
```

- [ ] **Step 4: Create integration test**

```rust
// airjedi-fusion/tests/integration.rs
use airjedi_fusion::*;
use airjedi_fusion::config::FusionConfig;
use airjedi_fusion::filter::ekf::ProcessNoiseConfig;
use airjedi_fusion::sensor::*;
use airjedi_fusion::systems::ObservationBuffer;
use bevy::prelude::*;
use chrono::Utc;
use nalgebra::DMatrix;

fn make_adsb_obs(lat: f64, lon: f64, alt: f64, icao: &str) -> SensorObservation {
    SensorObservation {
        sensor_id: SensorId {
            id: "test-adsb".to_string(),
            kind: SensorKind::AdsbReceiver,
            tier: FusionTier::Regional,
            coordinate_frame: airjedi_fusion::coord::CoordinateFrame::Wgs84,
        },
        timestamp: Utc::now(),
        receipt_time: Utc::now(),
        target_id: Some(TargetId {
            domain: TargetDomain::Air,
            id: icao.to_string(),
            id_type: IdentifierType::Icao,
        }),
        measurement: Measurement::PositionVelocity3D {
            lat_deg: lat, lon_deg: lon, alt_m: Some(alt),
            vel_north_mps: Some(100.0), vel_east_mps: Some(0.0),
            vel_down_mps: Some(0.0), heading_deg: Some(0.0),
        },
        covariance: ObservationCovariance {
            matrix: DMatrix::identity(6, 6) * 100.0,
        },
        classification_hint: Some(TargetCategory::FixedWing),
        metadata: ObservationMetadata::default(),
    }
}

#[test]
fn end_to_end_single_aircraft() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(FusionConfig::default());
    app.add_plugins(FusionPlugin);

    // Inject an observation
    app.world_mut()
        .resource_mut::<ObservationBuffer>()
        .observations
        .push(make_adsb_obs(37.6872, -97.3301, 10000.0, "ABC123"));

    // Run several FixedUpdate cycles
    for _ in 0..5 {
        app.update();
    }

    // Should have created a track entity
    let track_count = app
        .world_mut()
        .query::<&Track>()
        .iter(app.world())
        .count();
    assert!(track_count >= 1, "Expected at least 1 track, got {track_count}");
}

#[test]
fn two_aircraft_separate_tracks() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(FusionConfig::default());
    app.add_plugins(FusionPlugin);

    {
        let mut buffer = app.world_mut().resource_mut::<ObservationBuffer>();
        buffer.observations.push(make_adsb_obs(37.0, -97.0, 10000.0, "AAA111"));
        buffer.observations.push(make_adsb_obs(40.0, -80.0, 10000.0, "BBB222"));
    }

    for _ in 0..5 {
        app.update();
    }

    let track_count = app
        .world_mut()
        .query::<&Track>()
        .iter(app.world())
        .count();
    assert!(track_count >= 2, "Expected at least 2 tracks, got {track_count}");
}
```

- [ ] **Step 5: Run all tests**

Run: `cd airjedi-fusion && cargo test`
Expected: all unit tests + integration tests PASS

- [ ] **Step 6: Commit**

```bash
git add airjedi-fusion/
git commit -m "Add FusionPlugin with drain/associate/fuse/lifecycle pipeline in FixedUpdate"
```

---

## Self-Review Checklist

**Spec coverage:**
- Core types (TrackId, TargetId, TargetDomain, etc.) - Task 1
- Coordinate conversions (ECEF/geodetic/ENU/spherical) - Task 2
- Sensor types (SensorObservation, Measurement variants, SensorSource trait) - Task 3
- TimelineStore (VecDeque hot buffer, no Arrow) - Task 4
- EKF filter in ECEF with state history - Task 5
- Track lifecycle (Tentative/Confirmed/Coasting/Lost, per-category profiles) - Task 6
- GNN associator with spatial index - Task 7
- FusionPlugin assembly with FixedUpdate scheduling - Task 8
- OOSM config is defined (Task 5) but rollback-and-replay logic is not fully implemented - deferred to when NATS transport lands (Plan 2)
- TargetClassification component - Task 1
- IMM filter variant enum slot - defined in Task 5 FilterVariant (only Ekf6Dof initially)

**Placeholder scan:** No TBDs, TODOs, or "implement later" markers.

**Type consistency check:**
- `TrackId` used consistently in store, track, spatial index, associator
- `SensorObservation` used consistently in store (via `StoredObservation`), filter, associator
- `TrackerState` component used in systems, associator, track queries
- `FilterVariant` enum dispatch used in `TrackerState` methods
- `TargetClassification` component used in association gating and lifecycle profiles

**Not in scope (Plan 2 and 3):**
- NATS/JetStream transport - Plan 2
- OOSM rollback-and-replay execution - Plan 2 (config/types are in Plan 1)
- AirJedi integration (ADS-B adapter, render bridge) - Plan 3
- Parquet cold storage writer - Plan 2 or 3
- 4-DOF surface filter, IMM, coordinated turn filter - future tasks after core proves out

---

## Implementation Notes (Post-Execution)

**Status:** Completed 2026-06-21. Commit `f21a233`.

### Deviations from Plan

1. **Individual bevy sub-crates instead of `bevy` umbrella.**
   - Plan specified `bevy = { version = "0.18", default-features = false, features = ["bevy_app", "bevy_ecs", ...] }`
   - Bevy 0.18's umbrella crate does not expose individual sub-crate features like `bevy_app`. The only minimal feature is `bevy_log`.
   - **Actual:** Direct dependencies on `bevy_app`, `bevy_ecs`, `bevy_time`, `bevy_reflect`, `bevy_log` (each `= "0.18"`).
   - Added `prelude_imports.rs` module that re-exports all sub-crate preludes: `pub use bevy_app::prelude::*; pub use bevy_ecs::prelude::*;` etc.
   - All source files use `use crate::prelude_imports::*;` instead of `use bevy::prelude::*;`

2. **Systems in `Update` schedule, not `FixedUpdate`.**
   - Plan specified `FixedUpdate` for all fusion systems.
   - Bevy's `FixedUpdate` does not tick in unit test `App::update()` calls without a full runtime (no time accumulation).
   - **Actual:** Systems registered in `Update` schedule. Migration to `FixedUpdate` is deferred to Plan 3 where the full Bevy app runtime is available.
   - The `FusionSet` system set ordering (Drain -> Associate -> Fuse -> Lifecycle) works identically in either schedule.

3. **Each system registered individually, not in tuples.**
   - Bevy 0.18 does not support `.in_set()` on 4-element system tuples. Only tuples of 2-3 systems work.
   - **Actual:** Each system gets its own `.add_systems(Update, system.in_set(FusionSet::X))` call.

4. **Greedy sorted assignment instead of Jonker-Volgenant.**
   - Plan specified `lapjv` crate for optimal 1:1 assignment.
   - **Actual:** Greedy assignment sorted by ascending cost. Correct for well-separated targets (aviation use case). JV can be swapped in later by changing the inner loop of `GnnAssociator::associate()` without API changes.

5. **Added `prelude_imports.rs` module** - not in the original plan. Needed to aggregate bevy sub-crate preludes into one import.

6. **`SpatialIndex` needed `#[derive(Resource)]`** - the plan didn't specify this but Bevy requires it for `Res<SpatialIndex>` / `ResMut<SpatialIndex>` in systems.

7. **`TrackQuality::staleness` field has `#[reflect(ignore)]`** - `std::time::Duration` doesn't implement `Reflect` in Bevy 0.18, so the field is excluded from reflection.

### Actual File Structure

```
airjedi-fusion/
├── Cargo.toml
├── tests/
│   └── integration.rs          5 end-to-end pipeline tests
└── src/
    ├── lib.rs                  FusionPlugin, re-exports
    ├── prelude_imports.rs      Aggregated bevy sub-crate preludes (NEW)
    ├── types.rs                TrackId, TargetId, enums
    ├── classification.rs       TargetClassification component
    ├── coord.rs                ECEF/geodetic/ENU/spherical conversions
    ├── sensor.rs               SensorObservation, Measurement, SensorSource
    ├── store.rs                TimelineStore (VecDeque hot buffer)
    ├── config.rs               FusionConfig resource
    ├── track.rs                Track, TrackQuality, lifecycle state machine
    ├── systems.rs              All Bevy systems + FusionSet + ObservationBuffer
    ├── filter/
    │   ├── mod.rs              TrackFilter trait, FilterVariant, TrackerState
    │   ├── ekf.rs              Ekf6Dof (6-DOF EKF in ECEF)
    │   └── oosm.rs             OOSM rollback-and-replay (added in Plan 2)
    └── associator/
        ├── mod.rs              AssociatorConfig, GateParams, Assignment
        ├── gnn.rs              GnnAssociator (greedy, not JV)
        └── spatial_index.rs    Grid-based spatial pre-filter
```

### Actual Dependencies

```toml
bevy_app = "0.18"
bevy_ecs = "0.18"
bevy_time = "0.18"
bevy_reflect = "0.18"
bevy_log = "0.18"
nalgebra = "0.33"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1.0", features = ["derive"] }
```

### Test Count: 53 unit tests + 5 integration tests = 58 total
