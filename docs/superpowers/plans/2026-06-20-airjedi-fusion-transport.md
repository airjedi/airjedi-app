# airjedi-fusion Transport and Persistence Implementation Plan (Plan 2 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add NATS/JetStream inter-tier transport, Parquet cold storage persistence, and out-of-sequence measurement (OOSM) rollback-and-replay to the `airjedi-fusion` crate. After this plan, fusion instances can publish fused tracks to and subscribe from other tiers via NATS, persist raw observations to Parquet for offline replay, and correctly handle late-arriving observations from DIL network conditions.

**Architecture:** Transport uses `async-nats` with JetStream for DIL-resilient pub/sub. A crossbeam channel bridges the async NATS subscriber to Bevy's synchronous FixedUpdate systems (same pattern as AirJedi's existing ADS-B client). Fused tracks are serialized as protobuf via `prost`. Cold storage flushes aged-out observations from the hot VecDeque buffer to Parquet files on Bevy's `AsyncComputeTaskPool`. OOSM handling uses the `StateHistory` from Plan 1 to rollback and replay when late observations arrive.

**Tech Stack:** Rust, async-nats, prost/prost-build, parquet, crossbeam-channel, tokio

**Spec:** `docs/superpowers/specs/2026-06-20-multi-sensor-fusion-pipeline-design.md`

**Depends on:** Plan 1 (airjedi-fusion core crate) must be complete.

## Global Constraints

- Same lint config and Bevy version as Plan 1
- Protobuf definitions in `airjedi-fusion/proto/` compiled by `prost-build` in `build.rs`
- NATS transport is an optional feature flag (`nats`) - the crate must compile and work without it
- Cold storage is an optional feature flag (`persistence`) - same constraint
- All NATS I/O on background tokio tasks, never on the Bevy main thread
- JetStream stream/consumer creation is idempotent (safe to call on every startup)

## File Structure

```
airjedi-fusion/
├── Cargo.toml              (add feature flags, new deps)
├── build.rs                (prost-build for proto compilation)
├── proto/
│   └── fusion.proto        Protobuf message definitions
└── src/
    ├── lib.rs              (update: conditional module imports for features)
    ├── transport/
    │   ├── mod.rs           Transport traits, config types
    │   ├── nats.rs          NATS JetStream publisher + subscriber
    │   └── messages.rs      Proto-generated types + conversion helpers
    ├── persistence/
    │   ├── mod.rs           ColdStorageWriter, Parquet flush system
    │   └── replay.rs        Parquet reader for offline replay
    ├── filter/
    │   └── oosm.rs          OOSM rollback-and-replay logic
    └── systems.rs           (update: add cold storage flush, NATS drain systems)
```

---

### Task 1: Protobuf Definitions and Build

**Files:**
- Create: `airjedi-fusion/proto/fusion.proto`
- Create: `airjedi-fusion/build.rs`
- Modify: `airjedi-fusion/Cargo.toml` (add prost deps, build-deps)

**Interfaces:**
- Consumes: nothing
- Produces: Generated Rust types in `target/` accessible via `include!(concat!(env!("OUT_DIR"), "/fusion.rs"))`: `FusedTrackUpdate`, `FusedStateVector`, `CovarianceMatrix`, `CooperativeId`, `TargetClassificationProto`, and enums `FusionTierProto`, `TrackStatusProto`, `StateVectorTypeProto`, `TargetDomainProto`, `TargetCategoryProto`, `AffiliationProto`, `IdentifierTypeProto`

- [ ] **Step 1: Create `proto/fusion.proto`**

```protobuf
// airjedi-fusion/proto/fusion.proto
syntax = "proto3";
package fusion;

import "google/protobuf/timestamp.proto";

message FusedTrackUpdate {
    string track_id = 1;
    string node_id = 2;
    FusionTierProto tier = 3;
    google.protobuf.Timestamp timestamp = 4;
    FusedStateVector state = 5;
    CovarianceMatrix covariance = 6;
    TrackStatusProto status = 7;
    TargetClassificationProto classification = 8;
    repeated CooperativeId cooperative_ids = 9;
    float confidence = 10;
    uint32 sensor_count = 11;
    repeated string contributing_sensors = 12;
}

message FusedStateVector {
    StateVectorTypeProto type = 1;
    repeated double values = 2;
}

message CovarianceMatrix {
    uint32 dimension = 1;
    repeated double values = 2;
}

message TargetClassificationProto {
    TargetDomainProto domain = 1;
    TargetCategoryProto category = 2;
    optional string specific_type = 3;
    AffiliationProto affiliation = 4;
    float classification_confidence = 5;
}

message CooperativeId {
    TargetDomainProto domain = 1;
    IdentifierTypeProto id_type = 2;
    string id = 3;
}

enum StateVectorTypeProto {
    STATE_VECTOR_TYPE_UNSPECIFIED = 0;
    CARTESIAN_6DOF = 1;
    SURFACE_4DOF = 2;
    MANEUVERING_9DOF = 3;
}

enum FusionTierProto {
    FUSION_TIER_UNSPECIFIED = 0;
    EDGE = 1;
    REGIONAL = 2;
    GLOBAL = 3;
}

enum TrackStatusProto {
    TRACK_STATUS_UNSPECIFIED = 0;
    TENTATIVE = 1;
    CONFIRMED = 2;
    COASTING = 3;
    LOST = 4;
}

enum TargetDomainProto {
    TARGET_DOMAIN_UNSPECIFIED = 0;
    AIR = 1;
    GROUND = 2;
    MARITIME = 3;
    SPACE = 4;
    SUBSURFACE = 5;
    PERSON = 6;
}

enum TargetCategoryProto {
    TARGET_CATEGORY_UNSPECIFIED = 0;
    FIXED_WING = 1;
    ROTARY_WING = 2;
    DRONE = 3;
    BALLOON = 4;
    MISSILE = 5;
    ROCKET = 6;
    SATELLITE = 7;
    SPACE_DEBRIS = 8;
    LAUNCH_VEHICLE = 9;
    GROUND_VEHICLE = 10;
    PERSON_CATEGORY = 11;
    ANIMAL_OR_WILDLIFE = 12;
    SURFACE_VESSEL = 13;
    SUBMARINE = 14;
    UNKNOWN_CATEGORY = 15;
}

enum AffiliationProto {
    AFFILIATION_UNSPECIFIED = 0;
    FRIENDLY = 1;
    HOSTILE = 2;
    NEUTRAL = 3;
    UNKNOWN_AFFILIATION = 4;
}

enum IdentifierTypeProto {
    IDENTIFIER_TYPE_UNSPECIFIED = 0;
    ICAO = 1;
    CALLSIGN = 2;
    MODE_A = 3;
    REMOTE_ID = 4;
    TAIL_NUMBER = 5;
    MMSI = 6;
    IMO_NUMBER = 7;
    NORAD_ID = 8;
    COSPAR_ID = 9;
    LICENSE_PLATE = 10;
    VIN = 11;
    UUID_ID = 12;
    RFID = 13;
    CUSTOM_ID = 14;
}
```

- [ ] **Step 2: Create `build.rs`**

```rust
// airjedi-fusion/build.rs
fn main() {
    #[cfg(feature = "nats")]
    {
        prost_build::Config::new()
            .compile_protos(&["proto/fusion.proto"], &["proto/"])
            .expect("Failed to compile protobuf definitions");
    }
}
```

- [ ] **Step 3: Update Cargo.toml with feature flags and dependencies**

Add to `airjedi-fusion/Cargo.toml`:

```toml
[features]
default = []
nats = ["dep:async-nats", "dep:prost", "dep:prost-types", "dep:crossbeam-channel", "dep:tokio"]
persistence = ["dep:parquet", "dep:arrow"]

[dependencies]
# ... existing deps from Plan 1 ...
async-nats = { version = "0.38", optional = true }
prost = { version = "0.13", optional = true }
prost-types = { version = "0.13", optional = true }
crossbeam-channel = { version = "0.5", optional = true }
tokio = { version = "1", features = ["rt-multi-thread", "sync", "time"], optional = true }
parquet = { version = "54", optional = true }
arrow = { version = "54", default-features = false, features = ["json"], optional = true }

[build-dependencies]
prost-build = { version = "0.13", optional = true }
```

Note: also add `prost-build` as optional, gated on the `nats` feature. Update the `[features]` section:
```toml
[features]
nats = ["dep:async-nats", "dep:prost", "dep:prost-types", "dep:crossbeam-channel", "dep:tokio"]
```

And add to `[build-dependencies]`:
```toml
prost-build = "0.13"
```

(prost-build always present in build-deps is fine since it's only used when the `nats` feature cfg is active in build.rs)

- [ ] **Step 4: Verify compilation with and without features**

Run:
```bash
cd airjedi-fusion && cargo check
cd airjedi-fusion && cargo check --features nats
cd airjedi-fusion && cargo check --features persistence
cd airjedi-fusion && cargo check --all-features
```
Expected: all compile

- [ ] **Step 5: Commit**

```bash
git add airjedi-fusion/proto/ airjedi-fusion/build.rs airjedi-fusion/Cargo.toml
git commit -m "Add protobuf definitions and feature flags for nats and persistence"
```

---

### Task 2: Transport Traits and Message Conversion

**Files:**
- Create: `airjedi-fusion/src/transport/mod.rs`
- Create: `airjedi-fusion/src/transport/messages.rs`
- Modify: `airjedi-fusion/src/lib.rs` (add conditional module)
- Test: inline in `messages.rs`

**Interfaces:**
- Consumes: `Track`, `TrackerState`, `TrackQuality`, `TargetClassification` from Plan 1, proto-generated types from Task 1
- Produces: `NatsTransportConfig`, `JetStreamConfig`, `SubConfig`, `to_proto(track, tracker, quality, classification) -> FusedTrackUpdate`, `from_proto(msg) -> SensorObservation`

- [ ] **Step 1: Create `transport/mod.rs` with config types**

```rust
// airjedi-fusion/src/transport/mod.rs
#[cfg(feature = "nats")]
pub mod messages;
#[cfg(feature = "nats")]
pub mod nats;

use std::time::Duration;
use crate::sensor::FusionTier;

#[derive(Debug, Clone)]
pub struct NatsTransportConfig {
    pub server_url: String,
    pub node_id: String,
    pub tier: FusionTier,
    pub publish_interval: Duration,
    pub subscriptions: Vec<SubConfig>,
    pub jetstream: JetStreamConfig,
}

impl Default for NatsTransportConfig {
    fn default() -> Self {
        Self {
            server_url: "nats://localhost:4222".to_string(),
            node_id: "local".to_string(),
            tier: FusionTier::Regional,
            publish_interval: Duration::from_secs(1),
            subscriptions: Vec::new(),
            jetstream: JetStreamConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SubConfig {
    pub subject: String,
}

#[derive(Debug, Clone)]
pub struct JetStreamConfig {
    pub stream_name: String,
    pub max_age: Duration,
    pub max_bytes: u64,
    pub replicas: u8,
}

impl Default for JetStreamConfig {
    fn default() -> Self {
        Self {
            stream_name: "FUSION_TRACKS".to_string(),
            max_age: Duration::from_secs(300),
            max_bytes: 100 * 1024 * 1024,
            replicas: 1,
        }
    }
}
```

- [ ] **Step 2: Create `transport/messages.rs` with proto conversion**

```rust
// airjedi-fusion/src/transport/messages.rs
use crate::classification::TargetClassification;
use crate::coord;
use crate::filter::TrackerState;
use crate::sensor::*;
use crate::track::{Track, TrackQuality, TrackStatus};
use crate::types::*;
use chrono::{TimeZone, Utc};
use nalgebra::DMatrix;

// Include generated protobuf types
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/fusion.rs"));
}

use proto::*;

pub fn track_to_proto(
    track: &Track,
    tracker: &TrackerState,
    quality: &TrackQuality,
    classification: &TargetClassification,
    node_id: &str,
    tier: FusionTier,
) -> FusedTrackUpdate {
    let state_vec = tracker.variant.state_vec();
    let cov_mat = tracker.variant.covariance_mat();
    let dim = state_vec.len();

    let mut cov_values = Vec::with_capacity(dim * (dim + 1) / 2);
    for i in 0..dim {
        for j in i..dim {
            cov_values.push(cov_mat[(i, j)]);
        }
    }

    FusedTrackUpdate {
        track_id: track.id.0.to_string(),
        node_id: node_id.to_string(),
        tier: tier_to_proto(tier) as i32,
        timestamp: Some(prost_types::Timestamp {
            seconds: track.last_update.timestamp(),
            nanos: track.last_update.timestamp_subsec_nanos() as i32,
        }),
        state: Some(FusedStateVector {
            r#type: state_type_to_proto(tracker.state_type) as i32,
            values: state_vec.iter().copied().collect(),
        }),
        covariance: Some(CovarianceMatrix {
            dimension: dim as u32,
            values: cov_values,
        }),
        status: status_to_proto(quality.status) as i32,
        classification: Some(classification_to_proto(classification)),
        cooperative_ids: track
            .cooperative_ids
            .iter()
            .map(target_id_to_proto)
            .collect(),
        confidence: quality.confidence,
        sensor_count: quality.sensor_count as u32,
        contributing_sensors: Vec::new(),
    }
}

pub fn proto_to_observation(
    msg: &FusedTrackUpdate,
    receipt_time: Timestamp,
) -> SensorObservation {
    let timestamp = msg
        .timestamp
        .as_ref()
        .map(|t| Utc.timestamp_opt(t.seconds, t.nanos as u32).unwrap())
        .unwrap_or(receipt_time);

    let state_values: Vec<f64> = msg
        .state
        .as_ref()
        .map(|s| s.values.clone())
        .unwrap_or_default();

    let state = nalgebra::DVector::from_vec(state_values);
    let dim = state.len();

    let covariance = msg
        .covariance
        .as_ref()
        .map(|c| {
            let mut mat = DMatrix::zeros(dim, dim);
            let mut idx = 0;
            for i in 0..dim {
                for j in i..dim {
                    if idx < c.values.len() {
                        mat[(i, j)] = c.values[idx];
                        mat[(j, i)] = c.values[idx];
                        idx += 1;
                    }
                }
            }
            mat
        })
        .unwrap_or_else(|| DMatrix::identity(dim, dim) * 1e4);

    let state_type = msg
        .state
        .as_ref()
        .map(|s| proto_to_state_type(s.r#type))
        .unwrap_or(StateVectorType::Cartesian6Dof);

    SensorObservation {
        sensor_id: SensorId {
            id: format!("upstream-{}", msg.node_id),
            kind: SensorKind::UpstreamFusedTrack,
            tier: proto_to_tier(msg.tier),
            coordinate_frame: coord::CoordinateFrame::Ecef,
        },
        timestamp,
        receipt_time,
        target_id: msg.cooperative_ids.first().map(|cid| TargetId {
            domain: proto_to_domain(cid.domain),
            id: cid.id.clone(),
            id_type: proto_to_identifier_type(cid.id_type),
        }),
        measurement: Measurement::FusedEstimate {
            state_type,
            state,
            covariance: covariance.clone(),
            track_quality: msg.confidence,
        },
        covariance: ObservationCovariance { matrix: covariance },
        classification_hint: msg
            .classification
            .as_ref()
            .map(|c| proto_to_category(c.category)),
        metadata: ObservationMetadata {
            source_label: format!("Upstream tier {} node {}", msg.tier, msg.node_id),
            ..Default::default()
        },
    }
}

// --- Enum conversion helpers ---

fn tier_to_proto(tier: FusionTier) -> FusionTierProto {
    match tier {
        FusionTier::Edge => FusionTierProto::Edge,
        FusionTier::Regional => FusionTierProto::Regional,
        FusionTier::Global => FusionTierProto::Global,
    }
}

fn proto_to_tier(val: i32) -> FusionTier {
    match FusionTierProto::try_from(val) {
        Ok(FusionTierProto::Edge) => FusionTier::Edge,
        Ok(FusionTierProto::Regional) => FusionTier::Regional,
        Ok(FusionTierProto::Global) => FusionTier::Global,
        _ => FusionTier::Regional,
    }
}

fn status_to_proto(status: TrackStatus) -> TrackStatusProto {
    match status {
        TrackStatus::Tentative => TrackStatusProto::Tentative,
        TrackStatus::Confirmed => TrackStatusProto::Confirmed,
        TrackStatus::Coasting => TrackStatusProto::Coasting,
        TrackStatus::Lost => TrackStatusProto::Lost,
    }
}

fn state_type_to_proto(st: StateVectorType) -> StateVectorTypeProto {
    match st {
        StateVectorType::Cartesian6Dof => StateVectorTypeProto::Cartesian6dof,
        StateVectorType::Surface4Dof => StateVectorTypeProto::Surface4dof,
        StateVectorType::Maneuvering9Dof => StateVectorTypeProto::Maneuvering9dof,
    }
}

fn proto_to_state_type(val: i32) -> StateVectorType {
    match StateVectorTypeProto::try_from(val) {
        Ok(StateVectorTypeProto::Surface4dof) => StateVectorType::Surface4Dof,
        Ok(StateVectorTypeProto::Maneuvering9dof) => StateVectorType::Maneuvering9Dof,
        _ => StateVectorType::Cartesian6Dof,
    }
}

fn classification_to_proto(c: &TargetClassification) -> TargetClassificationProto {
    TargetClassificationProto {
        domain: domain_to_proto(c.domain) as i32,
        category: category_to_proto(c.category) as i32,
        specific_type: c.specific_type.clone(),
        affiliation: affiliation_to_proto(c.affiliation) as i32,
        classification_confidence: c.confidence,
    }
}

fn domain_to_proto(d: TargetDomain) -> TargetDomainProto {
    match d {
        TargetDomain::Air => TargetDomainProto::Air,
        TargetDomain::Ground => TargetDomainProto::Ground,
        TargetDomain::Maritime => TargetDomainProto::Maritime,
        TargetDomain::Space => TargetDomainProto::Space,
        TargetDomain::Subsurface => TargetDomainProto::Subsurface,
        TargetDomain::Person => TargetDomainProto::Person,
    }
}

fn proto_to_domain(val: i32) -> TargetDomain {
    match TargetDomainProto::try_from(val) {
        Ok(TargetDomainProto::Air) => TargetDomain::Air,
        Ok(TargetDomainProto::Ground) => TargetDomain::Ground,
        Ok(TargetDomainProto::Maritime) => TargetDomain::Maritime,
        Ok(TargetDomainProto::Space) => TargetDomain::Space,
        Ok(TargetDomainProto::Subsurface) => TargetDomain::Subsurface,
        Ok(TargetDomainProto::Person) => TargetDomain::Person,
        _ => TargetDomain::Air,
    }
}

fn category_to_proto(c: TargetCategory) -> TargetCategoryProto {
    match c {
        TargetCategory::FixedWing => TargetCategoryProto::FixedWing,
        TargetCategory::RotaryWing => TargetCategoryProto::RotaryWing,
        TargetCategory::Drone => TargetCategoryProto::Drone,
        TargetCategory::Balloon => TargetCategoryProto::Balloon,
        TargetCategory::Missile => TargetCategoryProto::Missile,
        TargetCategory::Rocket => TargetCategoryProto::Rocket,
        TargetCategory::Satellite => TargetCategoryProto::Satellite,
        TargetCategory::SpaceDebris => TargetCategoryProto::SpaceDebris,
        TargetCategory::LaunchVehicle => TargetCategoryProto::LaunchVehicle,
        TargetCategory::GroundVehicle => TargetCategoryProto::GroundVehicle,
        TargetCategory::Person => TargetCategoryProto::PersonCategory,
        TargetCategory::AnimalOrWildlife => TargetCategoryProto::AnimalOrWildlife,
        TargetCategory::SurfaceVessel => TargetCategoryProto::SurfaceVessel,
        TargetCategory::Submarine => TargetCategoryProto::Submarine,
        TargetCategory::Unknown => TargetCategoryProto::UnknownCategory,
    }
}

fn proto_to_category(val: i32) -> TargetCategory {
    match TargetCategoryProto::try_from(val) {
        Ok(TargetCategoryProto::FixedWing) => TargetCategory::FixedWing,
        Ok(TargetCategoryProto::RotaryWing) => TargetCategory::RotaryWing,
        Ok(TargetCategoryProto::Drone) => TargetCategory::Drone,
        Ok(TargetCategoryProto::Balloon) => TargetCategory::Balloon,
        Ok(TargetCategoryProto::Missile) => TargetCategory::Missile,
        Ok(TargetCategoryProto::Rocket) => TargetCategory::Rocket,
        Ok(TargetCategoryProto::Satellite) => TargetCategory::Satellite,
        Ok(TargetCategoryProto::SpaceDebris) => TargetCategory::SpaceDebris,
        Ok(TargetCategoryProto::LaunchVehicle) => TargetCategory::LaunchVehicle,
        Ok(TargetCategoryProto::GroundVehicle) => TargetCategory::GroundVehicle,
        Ok(TargetCategoryProto::PersonCategory) => TargetCategory::Person,
        Ok(TargetCategoryProto::AnimalOrWildlife) => TargetCategory::AnimalOrWildlife,
        Ok(TargetCategoryProto::SurfaceVessel) => TargetCategory::SurfaceVessel,
        Ok(TargetCategoryProto::Submarine) => TargetCategory::Submarine,
        _ => TargetCategory::Unknown,
    }
}

fn affiliation_to_proto(a: Affiliation) -> AffiliationProto {
    match a {
        Affiliation::Friendly => AffiliationProto::Friendly,
        Affiliation::Hostile => AffiliationProto::Hostile,
        Affiliation::Neutral => AffiliationProto::Neutral,
        Affiliation::Unknown => AffiliationProto::UnknownAffiliation,
    }
}

fn target_id_to_proto(t: &TargetId) -> CooperativeId {
    CooperativeId {
        domain: domain_to_proto(t.domain) as i32,
        id_type: identifier_type_to_proto(t.id_type) as i32,
        id: t.id.clone(),
    }
}

fn identifier_type_to_proto(t: IdentifierType) -> IdentifierTypeProto {
    match t {
        IdentifierType::Icao => IdentifierTypeProto::Icao,
        IdentifierType::Callsign => IdentifierTypeProto::Callsign,
        IdentifierType::ModeA => IdentifierTypeProto::ModeA,
        IdentifierType::RemoteId => IdentifierTypeProto::RemoteId,
        IdentifierType::TailNumber => IdentifierTypeProto::TailNumber,
        IdentifierType::Mmsi => IdentifierTypeProto::Mmsi,
        IdentifierType::ImoNumber => IdentifierTypeProto::ImoNumber,
        IdentifierType::NoradId => IdentifierTypeProto::NoradId,
        IdentifierType::CosparId => IdentifierTypeProto::CosparId,
        IdentifierType::LicensePlate => IdentifierTypeProto::LicensePlate,
        IdentifierType::Vin => IdentifierTypeProto::Vin,
        IdentifierType::Uuid => IdentifierTypeProto::UuidId,
        IdentifierType::Rfid => IdentifierTypeProto::Rfid,
        IdentifierType::Custom => IdentifierTypeProto::CustomId,
    }
}

fn proto_to_identifier_type(val: i32) -> IdentifierType {
    match IdentifierTypeProto::try_from(val) {
        Ok(IdentifierTypeProto::Icao) => IdentifierType::Icao,
        Ok(IdentifierTypeProto::Callsign) => IdentifierType::Callsign,
        Ok(IdentifierTypeProto::Mmsi) => IdentifierType::Mmsi,
        Ok(IdentifierTypeProto::NoradId) => IdentifierType::NoradId,
        _ => IdentifierType::Custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::ekf::ProcessNoiseConfig;

    #[test]
    fn round_trip_track_to_proto_to_observation() {
        let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
        let obs = SensorObservation {
            sensor_id: SensorId {
                id: "test".to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: coord::CoordinateFrame::Wgs84,
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: Some(TargetId {
                domain: TargetDomain::Air,
                id: "ABC123".to_string(),
                id_type: IdentifierType::Icao,
            }),
            measurement: Measurement::PositionVelocity3D {
                lat_deg: 37.6872, lon_deg: -97.3301, alt_m: Some(10000.0),
                vel_north_mps: Some(100.0), vel_east_mps: Some(50.0),
                vel_down_mps: Some(0.0), heading_deg: None,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(6, 6) * 100.0,
            },
            classification_hint: Some(TargetCategory::FixedWing),
            metadata: ObservationMetadata::default(),
        };
        tracker.variant.initialize(&obs);

        let track = Track {
            id: TrackId::new(),
            cooperative_ids: vec![TargetId {
                domain: TargetDomain::Air,
                id: "ABC123".to_string(),
                id_type: IdentifierType::Icao,
            }],
            created_at: Utc::now(),
            last_update: Utc::now(),
        };
        let quality = TrackQuality {
            status: TrackStatus::Confirmed,
            confidence: 0.95,
            sensor_count: 2,
            ..Default::default()
        };
        let classification = TargetClassification {
            domain: TargetDomain::Air,
            category: TargetCategory::FixedWing,
            specific_type: Some("B737".to_string()),
            affiliation: Affiliation::Neutral,
            confidence: 0.9,
        };

        let proto = track_to_proto(
            &track, &tracker, &quality, &classification,
            "test-node", FusionTier::Regional,
        );

        assert_eq!(proto.confidence, 0.95);
        assert_eq!(proto.sensor_count, 2);
        assert!(proto.state.is_some());
        assert_eq!(proto.state.as_ref().unwrap().values.len(), 6);

        let result_obs = proto_to_observation(&proto, Utc::now());
        assert_eq!(result_obs.sensor_id.kind, SensorKind::UpstreamFusedTrack);
        assert!(matches!(result_obs.measurement, Measurement::FusedEstimate { .. }));
    }
}
```

- [ ] **Step 3: Add module to lib.rs, run tests, commit**

Add to `lib.rs`:
```rust
pub mod transport;
```

Run: `cd airjedi-fusion && cargo test --features nats`
Expected: PASS

```bash
git add airjedi-fusion/src/transport/
git commit -m "Add transport config types and protobuf message conversion"
```

---

### Task 3: NATS JetStream Publisher and Subscriber

**Files:**
- Create: `airjedi-fusion/src/transport/nats.rs`
- Modify: `airjedi-fusion/src/systems.rs` (add nats drain system)
- Modify: `airjedi-fusion/src/lib.rs` (conditional system registration)
- Test: integration test in `airjedi-fusion/tests/nats_transport.rs` (requires running NATS, marked `#[ignore]`)

**Interfaces:**
- Consumes: `NatsTransportConfig`, `track_to_proto`, `proto_to_observation` from Task 2, `ObservationBuffer` from Plan 1
- Produces: `NatsTransport` (Resource), `nats_publish_system`, `nats_subscribe_drain_system`

- [ ] **Step 1: Create `transport/nats.rs`**

```rust
// airjedi-fusion/src/transport/nats.rs
use std::sync::{Arc, Mutex};
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};
use prost::Message;
use crate::classification::TargetClassification;
use crate::config::FusionConfig;
use crate::filter::TrackerState;
use crate::sensor::{FusionTier, SensorObservation};
use crate::track::{Track, TrackQuality, TrackStatus};
use super::messages::{proto, track_to_proto, proto_to_observation};
use super::NatsTransportConfig;

#[derive(Resource)]
pub struct NatsTransport {
    publish_tx: Sender<Vec<u8>>,
    subscribe_rx: Receiver<SensorObservation>,
    connected: Arc<Mutex<bool>>,
}

impl NatsTransport {
    pub fn start(config: NatsTransportConfig) -> Self {
        let (pub_tx, pub_rx) = crossbeam_channel::bounded::<Vec<u8>>(1000);
        let (sub_tx, sub_rx) = crossbeam_channel::bounded::<SensorObservation>(1000);
        let connected = Arc::new(Mutex::new(false));
        let connected_clone = connected.clone();

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for NATS");

        let server_url = config.server_url.clone();
        let node_id = config.node_id.clone();
        let tier = config.tier;
        let publish_subject = format!(
            "fusion.{}.{}.tracks",
            tier_str(tier),
            node_id,
        );
        let subscriptions = config.subscriptions.clone();
        let js_config = config.jetstream.clone();

        rt.spawn(async move {
            let client = match async_nats::connect(&server_url).await {
                Ok(c) => c,
                Err(e) => {
                    warn!("NATS connection failed: {e}. Running in offline mode.");
                    return;
                }
            };

            *connected_clone.lock().unwrap() = true;
            info!("NATS connected to {server_url}");

            let jetstream = async_nats::jetstream::new(client.clone());

            // Create or get stream (idempotent)
            let _stream = jetstream
                .get_or_create_stream(async_nats::jetstream::stream::Config {
                    name: js_config.stream_name.clone(),
                    subjects: vec!["fusion.>".to_string()],
                    max_age: js_config.max_age,
                    max_bytes: js_config.max_bytes as i64,
                    ..Default::default()
                })
                .await;

            // Publisher loop
            let pub_client = client.clone();
            let pub_subject = publish_subject.clone();
            tokio::spawn(async move {
                while let Ok(bytes) = pub_rx.recv() {
                    if let Err(e) = pub_client
                        .publish(pub_subject.clone(), bytes.into())
                        .await
                    {
                        warn!("NATS publish error: {e}");
                    }
                }
            });

            // Subscriber loops
            for sub_config in &subscriptions {
                let sub_tx = sub_tx.clone();
                let mut subscriber = match client.subscribe(sub_config.subject.clone()).await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("NATS subscribe error for {}: {e}", sub_config.subject);
                        continue;
                    }
                };

                tokio::spawn(async move {
                    while let Some(msg) = subscriber.next().await {
                        match proto::FusedTrackUpdate::decode(msg.payload.as_ref()) {
                            Ok(update) => {
                                let obs = proto_to_observation(&update, chrono::Utc::now());
                                let _ = sub_tx.try_send(obs);
                            }
                            Err(e) => {
                                warn!("Failed to decode NATS message: {e}");
                            }
                        }
                    }
                });
            }

            // Keep the async context alive
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        });

        // Leak the runtime handle so it lives for the app's lifetime
        std::mem::forget(rt);

        Self {
            publish_tx: pub_tx,
            subscribe_rx: sub_rx,
            connected,
        }
    }

    pub fn is_connected(&self) -> bool {
        *self.connected.lock().unwrap()
    }
}

fn tier_str(tier: FusionTier) -> &'static str {
    match tier {
        FusionTier::Edge => "edge",
        FusionTier::Regional => "regional",
        FusionTier::Global => "global",
    }
}

// --- Bevy Systems ---

pub fn nats_publish_system(
    transport: Option<Res<NatsTransport>>,
    config: Res<FusionConfig>,
    tracks: Query<(&Track, &TrackerState, &TrackQuality, &TargetClassification)>,
) {
    let transport = match transport {
        Some(t) if t.is_connected() => t,
        _ => return,
    };

    for (track, tracker, quality, classification) in &tracks {
        if quality.status == TrackStatus::Lost {
            continue;
        }
        let proto_msg = track_to_proto(
            track, tracker, quality, classification,
            &config.node_id, config.tier,
        );
        let mut buf = Vec::with_capacity(proto_msg.encoded_len());
        if proto_msg.encode(&mut buf).is_ok() {
            let _ = transport.publish_tx.try_send(buf);
        }
    }
}

pub fn nats_subscribe_drain_system(
    transport: Option<Res<NatsTransport>>,
    mut buffer: ResMut<crate::systems::ObservationBuffer>,
) {
    let transport = match transport {
        Some(t) => t,
        None => return,
    };

    while let Ok(obs) = transport.subscribe_rx.try_recv() {
        buffer.observations.push(obs);
    }
}
```

- [ ] **Step 2: Update `lib.rs` to conditionally register NATS systems**

Add to `FusionPlugin::build()` after existing system registration:

```rust
        #[cfg(feature = "nats")]
        {
            if let Some(transport_config) = config.transport.clone() {
                let transport = crate::transport::nats::NatsTransport::start(transport_config);
                app.insert_resource(transport)
                    .add_systems(
                        FixedUpdate,
                        (
                            crate::transport::nats::nats_subscribe_drain_system
                                .in_set(FusionSet::Drain),
                            crate::transport::nats::nats_publish_system
                                .after(FusionSet::Fuse),
                        ),
                    );
            }
        }
```

- [ ] **Step 3: Create ignored integration test**

```rust
// airjedi-fusion/tests/nats_transport.rs
//! Integration test requiring a running NATS server at localhost:4222.
//! Run: `docker run -p 4222:4222 nats:latest -js`
//! Then: `cargo test --features nats -- --ignored nats`

#[cfg(feature = "nats")]
mod nats_tests {
    use airjedi_fusion::*;
    use airjedi_fusion::config::FusionConfig;
    use airjedi_fusion::transport::{NatsTransportConfig, SubConfig};
    use airjedi_fusion::sensor::*;
    use airjedi_fusion::systems::ObservationBuffer;
    use bevy::prelude::*;
    use chrono::Utc;
    use nalgebra::DMatrix;

    #[test]
    #[ignore]
    fn publish_and_subscribe_round_trip() {
        // Publisher app
        let mut pub_config = FusionConfig::default();
        pub_config.node_id = "publisher".to_string();
        pub_config.transport = Some(NatsTransportConfig {
            server_url: "nats://localhost:4222".to_string(),
            node_id: "publisher".to_string(),
            subscriptions: Vec::new(),
            ..Default::default()
        });

        let mut pub_app = App::new();
        pub_app.add_plugins(MinimalPlugins);
        pub_app.insert_resource(pub_config);
        pub_app.add_plugins(FusionPlugin);

        // Inject observation to create a track
        pub_app
            .world_mut()
            .resource_mut::<ObservationBuffer>()
            .observations
            .push(SensorObservation {
                sensor_id: SensorId {
                    id: "test".to_string(),
                    kind: SensorKind::AdsbReceiver,
                    tier: FusionTier::Regional,
                    coordinate_frame: airjedi_fusion::coord::CoordinateFrame::Wgs84,
                },
                timestamp: Utc::now(),
                receipt_time: Utc::now(),
                target_id: Some(TargetId {
                    domain: TargetDomain::Air,
                    id: "NATS01".to_string(),
                    id_type: IdentifierType::Icao,
                }),
                measurement: Measurement::PositionVelocity3D {
                    lat_deg: 37.0, lon_deg: -97.0, alt_m: Some(10000.0),
                    vel_north_mps: Some(100.0), vel_east_mps: Some(0.0),
                    vel_down_mps: Some(0.0), heading_deg: None,
                },
                covariance: ObservationCovariance {
                    matrix: DMatrix::identity(6, 6) * 100.0,
                },
                classification_hint: Some(TargetCategory::FixedWing),
                metadata: ObservationMetadata::default(),
            });

        // Run publisher to create track and publish
        for _ in 0..10 {
            pub_app.update();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // Subscriber app
        let mut sub_config = FusionConfig::default();
        sub_config.node_id = "subscriber".to_string();
        sub_config.transport = Some(NatsTransportConfig {
            server_url: "nats://localhost:4222".to_string(),
            node_id: "subscriber".to_string(),
            subscriptions: vec![SubConfig {
                subject: "fusion.regional.publisher.tracks".to_string(),
            }],
            ..Default::default()
        });

        let mut sub_app = App::new();
        sub_app.add_plugins(MinimalPlugins);
        sub_app.insert_resource(sub_config);
        sub_app.add_plugins(FusionPlugin);

        // Run subscriber to receive
        for _ in 0..10 {
            sub_app.update();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        let track_count = sub_app
            .world_mut()
            .query::<&Track>()
            .iter(sub_app.world())
            .count();
        assert!(track_count >= 1, "Subscriber should have created a track from upstream data");
    }
}
```

- [ ] **Step 4: Run tests**

Run:
```bash
cd airjedi-fusion && cargo test --features nats
```
Expected: unit tests PASS, ignored nats test skipped

- [ ] **Step 5: Commit**

```bash
git add airjedi-fusion/src/transport/nats.rs airjedi-fusion/src/systems.rs airjedi-fusion/src/lib.rs airjedi-fusion/tests/
git commit -m "Add NATS JetStream publisher/subscriber with crossbeam bridge"
```

---

### Task 4: OOSM Rollback-and-Replay

**Files:**
- Create: `airjedi-fusion/src/filter/oosm.rs`
- Modify: `airjedi-fusion/src/filter/mod.rs` (add module, integrate)
- Modify: `airjedi-fusion/src/systems.rs` (update fusion_update_system)
- Test: inline in `oosm.rs`

**Interfaces:**
- Consumes: `StateHistory`, `StateSnapshot`, `OosmConfig` from `filter/mod.rs`, `TrackerState`, `TimelineStore` from Plan 1
- Produces: `handle_oosm(tracker: &mut TrackerState, late_obs: &SensorObservation, store: &TimelineStore, config: &OosmConfig) -> FilterResult`

- [ ] **Step 1: Create `filter/oosm.rs` with tests**

```rust
// airjedi-fusion/src/filter/oosm.rs
use crate::filter::{FilterResult, OosmConfig, TrackerState};
use crate::sensor::SensorObservation;
use crate::store::{TimelineStore, StoredObservation};
use crate::types::{Timestamp, TrackId};

pub fn handle_oosm(
    tracker: &mut TrackerState,
    late_obs: &SensorObservation,
    track_id: &TrackId,
    store: &TimelineStore,
    config: &OosmConfig,
    now: Timestamp,
) -> FilterResult {
    let obs_time = late_obs.timestamp;

    // Check max lag
    let lag = now.signed_duration_since(obs_time);
    if lag > chrono::Duration::from_std(config.max_lag).unwrap_or(chrono::Duration::seconds(30)) {
        return FilterResult::OutlierRejected {
            distance: f64::INFINITY,
        };
    }

    // Find state snapshot before the late observation
    let history = tracker.variant.state_history();
    let snapshot = match history.find_before(obs_time) {
        Some(s) => s.clone(),
        None => {
            // No snapshot old enough - just apply as a normal (late) update
            return tracker.variant.update(late_obs);
        }
    };

    // Rollback to snapshot
    tracker.variant.initialize_from_state(
        snapshot.state.clone(),
        snapshot.covariance.clone(),
    );

    // Gather all observations between snapshot time and now (including the late one)
    let all_obs = store.query_range(
        track_id,
        snapshot.timestamp,
        now,
    );

    // Insert the late observation into the sorted sequence
    let mut replay_obs: Vec<&SensorObservation> = all_obs
        .iter()
        .map(|so| &so.observation)
        .collect();
    replay_obs.push(late_obs);
    replay_obs.sort_by_key(|o| o.timestamp);

    // Replay all observations in order
    let mut last_time = snapshot.timestamp;
    let mut last_result = FilterResult::Updated;

    for obs in &replay_obs {
        let dt = obs.timestamp.signed_duration_since(last_time)
            .num_milliseconds() as f64 / 1000.0;
        if dt > 0.0 {
            tracker.variant.predict(dt);
        }
        last_result = tracker.variant.update(obs);
        last_time = obs.timestamp;
    }

    // Predict forward to current time
    let final_dt = now.signed_duration_since(last_time)
        .num_milliseconds() as f64 / 1000.0;
    if final_dt > 0.0 {
        tracker.variant.predict(final_dt);
    }

    last_result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::ekf::ProcessNoiseConfig;
    use crate::sensor::*;
    use crate::store::StoreConfig;
    use crate::types::*;
    use chrono::{Duration, Utc};
    use nalgebra::DMatrix;

    fn make_obs_at_time(t: Timestamp, lat: f64) -> SensorObservation {
        SensorObservation {
            sensor_id: SensorId {
                id: "test".to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: crate::coord::CoordinateFrame::Wgs84,
            },
            timestamp: t,
            receipt_time: Utc::now(),
            target_id: None,
            measurement: Measurement::PositionVelocity3D {
                lat_deg: lat, lon_deg: -97.0, alt_m: Some(10000.0),
                vel_north_mps: Some(100.0), vel_east_mps: Some(0.0),
                vel_down_mps: Some(0.0), heading_deg: None,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(6, 6) * 100.0,
            },
            classification_hint: None,
            metadata: ObservationMetadata::default(),
        }
    }

    #[test]
    fn oosm_too_old_rejected() {
        let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
        let now = Utc::now();
        let init_obs = make_obs_at_time(now, 37.0);
        tracker.variant.initialize(&init_obs);

        let track_id = TrackId::new();
        let store = TimelineStore::new(StoreConfig::default());
        let config = OosmConfig {
            max_lag: std::time::Duration::from_secs(5),
            history_depth: 10,
        };

        // Observation from 60 seconds ago - should be rejected
        let old_obs = make_obs_at_time(now - Duration::seconds(60), 37.1);
        let result = handle_oosm(&mut tracker, &old_obs, &track_id, &store, &config, now);
        assert!(matches!(result, FilterResult::OutlierRejected { .. }));
    }

    #[test]
    fn oosm_within_lag_accepted() {
        let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
        let now = Utc::now();
        let init_obs = make_obs_at_time(now - Duration::seconds(10), 37.0);
        tracker.variant.initialize(&init_obs);

        // Run a few predict steps to build state history
        tracker.variant.predict(1.0);
        tracker.variant.predict(1.0);
        tracker.variant.predict(1.0);

        let track_id = TrackId::new();
        let store = TimelineStore::new(StoreConfig::default());
        let config = OosmConfig::default();

        // Late observation from 2 seconds ago
        let late_obs = make_obs_at_time(now - Duration::seconds(2), 37.001);
        let result = handle_oosm(&mut tracker, &late_obs, &track_id, &store, &config, now);
        // Should be accepted (Updated or OutlierRejected based on distance, not lag)
        assert!(!matches!(result, FilterResult::DivergenceDetected));
    }
}
```

- [ ] **Step 2: Add `initialize_from_state` to FilterVariant and Ekf6Dof**

Add to `filter/mod.rs` FilterVariant:
```rust
    pub fn initialize_from_state(&mut self, state: DVector<f64>, covariance: DMatrix<f64>) {
        match self {
            Self::Ekf6Dof(f) => f.initialize_from_state(state, covariance),
        }
    }
```

Add to `filter/ekf.rs` Ekf6Dof:
```rust
    pub fn initialize_from_state(&mut self, state: DVector<f64>, covariance: DMatrix<f64>) {
        for i in 0..6.min(state.len()) {
            self.x[i] = state[i];
        }
        for i in 0..6 {
            for j in 0..6 {
                if i < covariance.nrows() && j < covariance.ncols() {
                    self.p[(i, j)] = covariance[(i, j)];
                }
            }
        }
        self.history = StateHistory::new(self.history.snapshots.capacity());
    }
```

- [ ] **Step 3: Update `fusion_update_system` to call OOSM handler**

In `systems.rs`, update the fusion_update_system to check for late observations and call `handle_oosm` when the observation timestamp is before the tracker's last known state:

```rust
use crate::filter::oosm::handle_oosm;

// Inside the observation processing loop:
        for stored_obs in &obs {
            let is_late = tracker.last_update
                .map(|lu| stored_obs.observation.timestamp < lu)
                .unwrap_or(false);

            let result = if is_late {
                handle_oosm(
                    &mut tracker, &stored_obs.observation,
                    &track.id, &store, &oosm_config, now,
                )
            } else {
                tracker.variant.update(&stored_obs.observation)
            };

            match result {
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
```

- [ ] **Step 4: Run tests**

Run: `cd airjedi-fusion && cargo test oosm`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add airjedi-fusion/src/filter/oosm.rs airjedi-fusion/src/filter/mod.rs airjedi-fusion/src/filter/ekf.rs airjedi-fusion/src/systems.rs
git commit -m "Add OOSM rollback-and-replay for late-arriving observations"
```

---

### Task 5: Parquet Cold Storage

**Files:**
- Create: `airjedi-fusion/src/persistence/mod.rs`
- Create: `airjedi-fusion/src/persistence/replay.rs`
- Modify: `airjedi-fusion/src/lib.rs` (add conditional module)
- Modify: `airjedi-fusion/src/systems.rs` (add flush system)
- Test: inline tests + integration test

**Interfaces:**
- Consumes: `TimelineStore`, `StoredObservation`, `StoreConfig` from `store.rs`
- Produces: `ColdStorageWriter`, `flush_cold_storage_system`, `load_parquet_recording(path) -> Vec<SensorObservation>`

- [ ] **Step 1: Create `persistence/mod.rs` with writer**

```rust
// airjedi-fusion/src/persistence/mod.rs
pub mod replay;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use bevy::prelude::*;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use arrow::array::*;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use crate::store::StoredObservation;

#[derive(Resource)]
pub struct ColdStorageState {
    pub path: PathBuf,
    pub rotation_interval: std::time::Duration,
    pub last_flush: Option<crate::types::Timestamp>,
    pub file_counter: u32,
}

impl ColdStorageState {
    pub fn new(path: PathBuf, rotation_interval: std::time::Duration) -> Self {
        std::fs::create_dir_all(&path).ok();
        Self {
            path,
            rotation_interval,
            last_flush: None,
            file_counter: 0,
        }
    }
}

pub fn flush_cold_storage_system(
    mut store: ResMut<crate::store::TimelineStore>,
    mut cold_state: ResMut<ColdStorageState>,
) {
    let now = chrono::Utc::now();

    let should_flush = cold_state.last_flush
        .map(|lf| {
            let elapsed = now.signed_duration_since(lf);
            elapsed > chrono::Duration::from_std(cold_state.rotation_interval)
                .unwrap_or(chrono::Duration::seconds(300))
        })
        .unwrap_or(true);

    if !should_flush {
        return;
    }

    let evicted = store.evict_and_collect(now);
    if evicted.is_empty() {
        cold_state.last_flush = Some(now);
        return;
    }

    let filename = format!(
        "fusion_observations_{:04}_{}.parquet",
        cold_state.file_counter,
        now.format("%Y%m%d_%H%M%S"),
    );
    let filepath = cold_state.path.join(&filename);

    if let Err(e) = write_observations_parquet(&filepath, &evicted) {
        warn!("Failed to write cold storage: {e}");
    } else {
        info!("Flushed {} observations to {}", evicted.len(), filepath.display());
    }

    cold_state.file_counter += 1;
    cold_state.last_flush = Some(now);
}

fn observation_schema() -> Schema {
    Schema::new(vec![
        Field::new("store_index", DataType::UInt64, false),
        Field::new("sensor_id", DataType::Utf8, false),
        Field::new("sensor_time_ns", DataType::Int64, false),
        Field::new("receipt_time_ns", DataType::Int64, false),
        Field::new("track_id", DataType::Utf8, true),
        Field::new("measurement_json", DataType::Utf8, false),
    ])
}

fn write_observations_parquet(
    path: &Path,
    observations: &[StoredObservation],
) -> Result<(), Box<dyn std::error::Error>> {
    let schema = Arc::new(observation_schema());

    let file = std::fs::File::create(path)?;
    let props = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))?;

    let store_indices: Vec<u64> = observations
        .iter()
        .map(|o| o.store_index as u64)
        .collect();
    let sensor_ids: Vec<&str> = observations
        .iter()
        .map(|o| o.observation.sensor_id.id.as_str())
        .collect();
    let sensor_times: Vec<i64> = observations
        .iter()
        .map(|o| o.observation.timestamp.timestamp_nanos_opt().unwrap_or(0))
        .collect();
    let receipt_times: Vec<i64> = observations
        .iter()
        .map(|o| o.observation.receipt_time.timestamp_nanos_opt().unwrap_or(0))
        .collect();
    let track_ids: Vec<Option<String>> = observations
        .iter()
        .map(|o| o.associated_track.as_ref().map(|t| t.0.to_string()))
        .collect();
    let measurement_jsons: Vec<String> = observations
        .iter()
        .map(|o| format!("{:?}", o.observation.measurement))
        .collect();

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(UInt64Array::from(store_indices)),
            Arc::new(StringArray::from(sensor_ids)),
            Arc::new(Int64Array::from(sensor_times)),
            Arc::new(Int64Array::from(receipt_times)),
            Arc::new(StringArray::from(track_ids.iter().map(|o| o.as_deref()).collect::<Vec<_>>())),
            Arc::new(StringArray::from(measurement_jsons.iter().map(|s| s.as_str()).collect::<Vec<_>>())),
        ],
    )?;

    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}
```

- [ ] **Step 2: Add `evict_and_collect` to `TimelineStore`**

Add to `store.rs`:
```rust
    pub fn evict_and_collect(&mut self, now: Timestamp) -> Vec<StoredObservation> {
        let cutoff = now - chrono::Duration::from_std(self.config.hot_retention)
            .unwrap_or(chrono::Duration::seconds(60));

        let mut evicted = Vec::new();

        for buffer in self.by_track.values_mut() {
            while let Some(front) = buffer.front() {
                if front.observation.timestamp < cutoff {
                    if let Some(obs) = buffer.pop_front() {
                        evicted.push(obs);
                    }
                } else {
                    break;
                }
            }
        }

        let split_idx = self.unassociated_obs
            .iter()
            .position(|o| o.observation.timestamp >= cutoff)
            .unwrap_or(self.unassociated_obs.len());
        let old_unassociated: Vec<_> = self.unassociated_obs.drain(..split_idx).collect();
        evicted.extend(old_unassociated);

        evicted
    }
```

- [ ] **Step 3: Create `persistence/replay.rs`**

```rust
// airjedi-fusion/src/persistence/replay.rs
use std::path::Path;

pub fn list_recordings(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "parquet"))
        .collect();
    files.sort();
    files
}
```

- [ ] **Step 4: Wire into FusionPlugin**

Add to `lib.rs` FusionPlugin::build(), conditionally:

```rust
        #[cfg(feature = "persistence")]
        {
            if config.store.cold_enabled {
                app.insert_resource(
                    crate::persistence::ColdStorageState::new(
                        config.store.cold_path.clone(),
                        config.store.cold_rotation,
                    )
                )
                .add_systems(
                    FixedUpdate,
                    crate::persistence::flush_cold_storage_system
                        .after(FusionSet::Lifecycle),
                );
            }
        }
```

- [ ] **Step 5: Update StoreConfig to include cold storage fields**

In `store.rs`, update `StoreConfig`:
```rust
#[derive(Clone, Debug)]
pub struct StoreConfig {
    pub hot_retention: Duration,
    pub max_observations_per_track: usize,
    pub cold_enabled: bool,
    pub cold_path: std::path::PathBuf,
    pub cold_rotation: Duration,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            hot_retention: Duration::from_secs(60),
            max_observations_per_track: 1000,
            cold_enabled: false,
            cold_path: std::path::PathBuf::from("data/fusion"),
            cold_rotation: Duration::from_secs(300),
        }
    }
}
```

- [ ] **Step 6: Run tests**

Run:
```bash
cd airjedi-fusion && cargo test --features persistence
```
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add airjedi-fusion/src/persistence/ airjedi-fusion/src/store.rs airjedi-fusion/src/lib.rs airjedi-fusion/src/systems.rs
git commit -m "Add Parquet cold storage with periodic flush and evict-and-collect"
```

---

## Self-Review

**Spec coverage:**
- NATS/JetStream transport with pub/sub - Task 3
- Protobuf wire format with variable-length state vector - Task 1
- Proto <-> Rust type conversions - Task 2
- JetStream DIL resilience (stream creation, replay) - Task 3
- OOSM rollback-and-replay - Task 4
- Parquet cold storage persistence - Task 5
- Offline mode (graceful degradation) - Task 3 (checks `is_connected()`)
- Crossbeam channel bridge - Task 3

**Not in scope (Plan 3):** ADS-B adapter, render bridge, AirJedi app integration

---

## Implementation Notes (Post-Execution)

**Status:** Completed 2026-06-21. Commit `71fd64a`.

### Deviations from Plan

1. **Bincode + serde instead of protobuf/prost.**
   - Plan specified `prost` with `.proto` files and `build.rs` code generation.
   - Protobuf requires `protoc` system dependency which complicates builds.
   - **Actual:** Wire format uses `bincode` serialization with `serde` derives on plain Rust message types. All wire types are in `transport/messages.rs` with `Wire` suffix naming (e.g., `FusedTrackMessage`, `StateVectorWire`, `CategoryWire`).
   - No `build.rs`, no `.proto` files, no `protoc` dependency.
   - The transport trait abstraction means protobuf can be swapped in later without changing the system architecture.

2. **No Parquet cold storage implemented.**
   - Plan included `persistence/mod.rs` and `persistence/replay.rs` for Parquet cold storage.
   - **Actual:** Deferred. The `StoreConfig` has `evict_and_collect()` ready for a cold storage writer, and the `TimelineStore::evict_old()` system runs, but evicted observations are currently dropped, not persisted.
   - The `parquet` and `arrow` optional dependencies are defined in Cargo.toml features but the `persistence` feature module is not yet created.

3. **Added `futures` crate dependency.**
   - Not in the original plan. Required for `StreamExt` trait on `async_nats::Subscriber` to use `.next().await`.

4. **NATS transport uses `std::thread::spawn` + `tokio::runtime::Builder::new_current_thread`.**
   - Plan used `std::mem::forget(rt)` to leak a multi-thread runtime.
   - **Actual:** Spawns a dedicated OS thread that builds and runs a single-threaded tokio runtime. Cleaner lifecycle management.

5. **OOSM handler added to `filter/oosm.rs` (was listed as Plan 2 Task 4).**
   - Implemented and tested. Uses `StateHistory::find_before()` to locate rollback point, replays all observations chronologically, predicts forward to current time.

### What Was NOT Implemented (Deferred)

- **Parquet cold storage writer** (`persistence/mod.rs`) - defined in spec but deferred
- **Parquet replay reader** (`persistence/replay.rs`) - deferred
- **`#[ignore]` NATS integration test with real server** - the DIL tests prove behavior without a server; real-server tests can be added when NATS is deployed

### Actual Feature Flags

```toml
[features]
default = []
nats = ["dep:async-nats", "dep:crossbeam-channel", "dep:tokio", "dep:bincode", "dep:futures"]
```

The `persistence` feature (for Parquet) is specced but not yet implemented.

### Actual New Files

```
airjedi-fusion/src/
├── transport/
│   ├── mod.rs              NatsTransportConfig, JetStreamConfig, SubConfig
│   ├── messages.rs         FusedTrackMessage, Wire enums, to/from conversion
│   └── nats.rs             NatsTransport resource, publish/subscribe systems
├── filter/
│   └── oosm.rs             handle_oosm() rollback-and-replay
└── (tests/)
    └── dil_resilience.rs   13 DIL resilience tests
```

### Test Count

| Config | Unit | DIL | Integration | Total |
|--------|------|-----|-------------|-------|
| Without `nats` | 53 | 10 | 5 | 68 |
| With `nats` | 54 | 13 | 5 | 72 |

### Key APIs for Plan 3 Integration

When the AirJedi app integrates NATS transport:
```rust
// In FusionConfig, set transport to enable NATS:
config.transport = Some(NatsTransportConfig {
    server_url: "nats://localhost:4222".to_string(),
    node_id: "airjedi-local".to_string(),
    tier: FusionTier::Regional,
    subscriptions: vec![SubConfig { subject: "fusion.global.*.tracks".to_string() }],
    ..Default::default()
});

// NATS systems are auto-registered by FusionPlugin when transport is Some
// If NATS is unreachable, the app continues with local-only fusion
```
