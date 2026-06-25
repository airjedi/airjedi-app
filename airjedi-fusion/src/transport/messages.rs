use crate::classification::TargetClassification;
use crate::coord;
use crate::filter::TrackerState;
use crate::sensor::*;
use crate::track::{Track, TrackQuality, TrackStatus};
use crate::types::*;
use chrono::{TimeZone, Utc};
use nalgebra::DMatrix;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FusedTrackMessage {
    pub track_id: String,
    pub node_id: String,
    pub tier: TierWire,
    pub timestamp_ms: i64,
    pub state: StateVectorWire,
    pub covariance: CovarianceWire,
    pub status: StatusWire,
    pub classification: ClassificationWire,
    pub cooperative_ids: Vec<CooperativeIdWire>,
    pub confidence: f32,
    pub sensor_count: u8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TierWire {
    Edge,
    Regional,
    Global,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateVectorWire {
    pub state_type: StateTypeWire,
    pub values: Vec<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StateTypeWire {
    Cartesian6Dof,
    Surface4Dof,
    Maneuvering9Dof,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CovarianceWire {
    pub dimension: usize,
    pub upper_triangle: Vec<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StatusWire {
    Tentative,
    Confirmed,
    Coasting,
    Lost,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationWire {
    pub domain: DomainWire,
    pub category: CategoryWire,
    pub specific_type: Option<String>,
    pub affiliation: AffiliationWire,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DomainWire {
    Air,
    Ground,
    Maritime,
    Space,
    Subsurface,
    Person,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CategoryWire {
    FixedWing,
    RotaryWing,
    Drone,
    Balloon,
    Missile,
    Rocket,
    Satellite,
    SpaceDebris,
    LaunchVehicle,
    GroundVehicle,
    Person,
    AnimalOrWildlife,
    SurfaceVessel,
    Submarine,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AffiliationWire {
    Friendly,
    Hostile,
    Neutral,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CooperativeIdWire {
    pub domain: DomainWire,
    pub id_type: IdTypeWire,
    pub id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IdTypeWire {
    Icao,
    Callsign,
    ModeA,
    RemoteId,
    TailNumber,
    Mmsi,
    ImoNumber,
    NoradId,
    CosparId,
    LicensePlate,
    Vin,
    Uuid,
    Rfid,
    Custom,
}

// --- Conversion: internal types -> wire ---

pub fn track_to_message(
    track: &Track,
    tracker: &TrackerState,
    quality: &TrackQuality,
    classification: &TargetClassification,
    node_id: &str,
    tier: FusionTier,
) -> FusedTrackMessage {
    let state_vec = tracker.variant.state_vec();
    let cov_mat = tracker.variant.covariance_mat();
    let dim = state_vec.len();

    let mut upper_tri = Vec::with_capacity(dim * (dim + 1) / 2);
    for i in 0..dim {
        for j in i..dim {
            upper_tri.push(cov_mat[(i, j)]);
        }
    }

    FusedTrackMessage {
        track_id: track.id.0.to_string(),
        node_id: node_id.to_string(),
        tier: tier_to_wire(tier),
        timestamp_ms: track.last_update.timestamp_millis(),
        state: StateVectorWire {
            state_type: state_type_to_wire(tracker.state_type),
            values: state_vec.iter().copied().collect(),
        },
        covariance: CovarianceWire {
            dimension: dim,
            upper_triangle: upper_tri,
        },
        status: status_to_wire(quality.status),
        classification: classification_to_wire(classification),
        cooperative_ids: track
            .cooperative_ids
            .iter()
            .map(target_id_to_wire)
            .collect(),
        confidence: quality.confidence,
        sensor_count: quality.sensor_count,
    }
}

// --- Conversion: wire -> SensorObservation (for local fusion) ---

pub fn message_to_observation(
    msg: &FusedTrackMessage,
    receipt_time: Timestamp,
) -> SensorObservation {
    let timestamp = Utc
        .timestamp_millis_opt(msg.timestamp_ms)
        .single()
        .unwrap_or(receipt_time);

    let state = nalgebra::DVector::from_vec(msg.state.values.clone());
    let dim = msg.covariance.dimension;

    let mut cov = DMatrix::zeros(dim, dim);
    let mut idx = 0;
    for i in 0..dim {
        for j in i..dim {
            if idx < msg.covariance.upper_triangle.len() {
                cov[(i, j)] = msg.covariance.upper_triangle[idx];
                cov[(j, i)] = msg.covariance.upper_triangle[idx];
                idx += 1;
            }
        }
    }

    SensorObservation {
        sensor_id: SensorId {
            id: format!("upstream-{}", msg.node_id),
            kind: SensorKind::UpstreamFusedTrack,
            tier: wire_to_tier(msg.tier),
            coordinate_frame: coord::CoordinateFrame::Ecef,
        },
        timestamp,
        receipt_time,
        target_id: msg.cooperative_ids.first().map(|cid| TargetId {
            domain: wire_to_domain(cid.domain),
            id: cid.id.clone(),
            id_type: wire_to_id_type(cid.id_type),
        }),
        measurement: Measurement::FusedEstimate {
            state_type: wire_to_state_type(msg.state.state_type),
            state: state.clone(),
            covariance: cov.clone(),
            track_quality: msg.confidence,
        },
        covariance: ObservationCovariance { matrix: cov },
        classification_hint: Some(wire_to_category(msg.classification.category)),
        metadata: ObservationMetadata {
            source_label: format!("Upstream {} node {}", tier_label(msg.tier), msg.node_id),
            ..Default::default()
        },
    }
}

// --- Enum conversion helpers ---

fn tier_to_wire(t: FusionTier) -> TierWire {
    match t {
        FusionTier::Edge => TierWire::Edge,
        FusionTier::Regional => TierWire::Regional,
        FusionTier::Global => TierWire::Global,
    }
}

fn wire_to_tier(t: TierWire) -> FusionTier {
    match t {
        TierWire::Edge => FusionTier::Edge,
        TierWire::Regional => FusionTier::Regional,
        TierWire::Global => FusionTier::Global,
    }
}

fn state_type_to_wire(s: StateVectorType) -> StateTypeWire {
    match s {
        StateVectorType::Cartesian6Dof => StateTypeWire::Cartesian6Dof,
        StateVectorType::Surface4Dof => StateTypeWire::Surface4Dof,
        StateVectorType::Maneuvering9Dof => StateTypeWire::Maneuvering9Dof,
    }
}

fn wire_to_state_type(s: StateTypeWire) -> StateVectorType {
    match s {
        StateTypeWire::Cartesian6Dof => StateVectorType::Cartesian6Dof,
        StateTypeWire::Surface4Dof => StateVectorType::Surface4Dof,
        StateTypeWire::Maneuvering9Dof => StateVectorType::Maneuvering9Dof,
    }
}

fn status_to_wire(s: TrackStatus) -> StatusWire {
    match s {
        TrackStatus::Tentative => StatusWire::Tentative,
        TrackStatus::Confirmed => StatusWire::Confirmed,
        TrackStatus::Coasting => StatusWire::Coasting,
        TrackStatus::Lost => StatusWire::Lost,
    }
}

fn classification_to_wire(c: &TargetClassification) -> ClassificationWire {
    ClassificationWire {
        domain: domain_to_wire(c.domain),
        category: category_to_wire(c.category),
        specific_type: c.specific_type.clone(),
        affiliation: affiliation_to_wire(c.affiliation),
        confidence: c.confidence,
    }
}

fn domain_to_wire(d: TargetDomain) -> DomainWire {
    match d {
        TargetDomain::Air => DomainWire::Air,
        TargetDomain::Ground => DomainWire::Ground,
        TargetDomain::Maritime => DomainWire::Maritime,
        TargetDomain::Space => DomainWire::Space,
        TargetDomain::Subsurface => DomainWire::Subsurface,
        TargetDomain::Person => DomainWire::Person,
    }
}

fn wire_to_domain(d: DomainWire) -> TargetDomain {
    match d {
        DomainWire::Air => TargetDomain::Air,
        DomainWire::Ground => TargetDomain::Ground,
        DomainWire::Maritime => TargetDomain::Maritime,
        DomainWire::Space => TargetDomain::Space,
        DomainWire::Subsurface => TargetDomain::Subsurface,
        DomainWire::Person => TargetDomain::Person,
    }
}

fn category_to_wire(c: TargetCategory) -> CategoryWire {
    match c {
        TargetCategory::FixedWing => CategoryWire::FixedWing,
        TargetCategory::RotaryWing => CategoryWire::RotaryWing,
        TargetCategory::Drone => CategoryWire::Drone,
        TargetCategory::Balloon => CategoryWire::Balloon,
        TargetCategory::Missile => CategoryWire::Missile,
        TargetCategory::Rocket => CategoryWire::Rocket,
        TargetCategory::Satellite => CategoryWire::Satellite,
        TargetCategory::SpaceDebris => CategoryWire::SpaceDebris,
        TargetCategory::LaunchVehicle => CategoryWire::LaunchVehicle,
        TargetCategory::GroundVehicle => CategoryWire::GroundVehicle,
        TargetCategory::Person => CategoryWire::Person,
        TargetCategory::AnimalOrWildlife => CategoryWire::AnimalOrWildlife,
        TargetCategory::SurfaceVessel => CategoryWire::SurfaceVessel,
        TargetCategory::Submarine => CategoryWire::Submarine,
        TargetCategory::Unknown => CategoryWire::Unknown,
    }
}

fn wire_to_category(c: CategoryWire) -> TargetCategory {
    match c {
        CategoryWire::FixedWing => TargetCategory::FixedWing,
        CategoryWire::RotaryWing => TargetCategory::RotaryWing,
        CategoryWire::Drone => TargetCategory::Drone,
        CategoryWire::Balloon => TargetCategory::Balloon,
        CategoryWire::Missile => TargetCategory::Missile,
        CategoryWire::Rocket => TargetCategory::Rocket,
        CategoryWire::Satellite => TargetCategory::Satellite,
        CategoryWire::SpaceDebris => TargetCategory::SpaceDebris,
        CategoryWire::LaunchVehicle => TargetCategory::LaunchVehicle,
        CategoryWire::GroundVehicle => TargetCategory::GroundVehicle,
        CategoryWire::Person => TargetCategory::Person,
        CategoryWire::AnimalOrWildlife => TargetCategory::AnimalOrWildlife,
        CategoryWire::SurfaceVessel => TargetCategory::SurfaceVessel,
        CategoryWire::Submarine => TargetCategory::Submarine,
        CategoryWire::Unknown => TargetCategory::Unknown,
    }
}

fn affiliation_to_wire(a: Affiliation) -> AffiliationWire {
    match a {
        Affiliation::Friendly => AffiliationWire::Friendly,
        Affiliation::Hostile => AffiliationWire::Hostile,
        Affiliation::Neutral => AffiliationWire::Neutral,
        Affiliation::Unknown => AffiliationWire::Unknown,
    }
}

fn target_id_to_wire(t: &TargetId) -> CooperativeIdWire {
    CooperativeIdWire {
        domain: domain_to_wire(t.domain),
        id_type: id_type_to_wire(t.id_type),
        id: t.id.clone(),
    }
}

fn id_type_to_wire(t: IdentifierType) -> IdTypeWire {
    match t {
        IdentifierType::Icao => IdTypeWire::Icao,
        IdentifierType::Callsign => IdTypeWire::Callsign,
        IdentifierType::ModeA => IdTypeWire::ModeA,
        IdentifierType::RemoteId => IdTypeWire::RemoteId,
        IdentifierType::TailNumber => IdTypeWire::TailNumber,
        IdentifierType::Mmsi => IdTypeWire::Mmsi,
        IdentifierType::ImoNumber => IdTypeWire::ImoNumber,
        IdentifierType::NoradId => IdTypeWire::NoradId,
        IdentifierType::CosparId => IdTypeWire::CosparId,
        IdentifierType::LicensePlate => IdTypeWire::LicensePlate,
        IdentifierType::Vin => IdTypeWire::Vin,
        IdentifierType::Uuid => IdTypeWire::Uuid,
        IdentifierType::Rfid => IdTypeWire::Rfid,
        IdentifierType::Custom => IdTypeWire::Custom,
    }
}

fn wire_to_id_type(t: IdTypeWire) -> IdentifierType {
    match t {
        IdTypeWire::Icao => IdentifierType::Icao,
        IdTypeWire::Callsign => IdentifierType::Callsign,
        IdTypeWire::ModeA => IdentifierType::ModeA,
        IdTypeWire::RemoteId => IdentifierType::RemoteId,
        IdTypeWire::TailNumber => IdentifierType::TailNumber,
        IdTypeWire::Mmsi => IdentifierType::Mmsi,
        IdTypeWire::ImoNumber => IdentifierType::ImoNumber,
        IdTypeWire::NoradId => IdentifierType::NoradId,
        IdTypeWire::CosparId => IdentifierType::CosparId,
        IdTypeWire::LicensePlate => IdentifierType::LicensePlate,
        IdTypeWire::Vin => IdentifierType::Vin,
        IdTypeWire::Uuid => IdentifierType::Uuid,
        IdTypeWire::Rfid => IdentifierType::Rfid,
        IdTypeWire::Custom => IdentifierType::Custom,
    }
}

fn tier_label(t: TierWire) -> &'static str {
    match t {
        TierWire::Edge => "edge",
        TierWire::Regional => "regional",
        TierWire::Global => "global",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::ekf::ProcessNoiseConfig;
    use approx::assert_relative_eq;

    fn make_test_track() -> (Track, TrackerState, TrackQuality, TargetClassification) {
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
                lat_deg: 37.6872,
                lon_deg: -97.3301,
                alt_m: Some(10000.0),
                vel_north_mps: Some(100.0),
                vel_east_mps: Some(50.0),
                vel_down_mps: Some(0.0),
                heading_deg: None,
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
            is_on_ground: false,
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

        (track, tracker, quality, classification)
    }

    #[test]
    fn round_trip_message_preserves_state() {
        let (track, tracker, quality, classification) = make_test_track();

        let msg = track_to_message(
            &track,
            &tracker,
            &quality,
            &classification,
            "test-node",
            FusionTier::Regional,
        );

        assert_eq!(msg.confidence, 0.95);
        assert_eq!(msg.sensor_count, 2);
        assert_eq!(msg.state.values.len(), 6);
        assert_eq!(msg.status, StatusWire::Confirmed);
        assert_eq!(msg.classification.category, CategoryWire::FixedWing);
        assert_eq!(msg.cooperative_ids.len(), 1);
        assert_eq!(msg.cooperative_ids[0].id, "ABC123");

        // Convert back to observation
        let obs = message_to_observation(&msg, Utc::now());
        assert_eq!(obs.sensor_id.kind, SensorKind::UpstreamFusedTrack);
        assert!(matches!(obs.measurement, Measurement::FusedEstimate { .. }));
        assert_eq!(obs.target_id.as_ref().unwrap().id, "ABC123");
    }

    #[test]
    fn covariance_round_trip() {
        let (track, tracker, quality, classification) = make_test_track();
        let msg = track_to_message(
            &track,
            &tracker,
            &quality,
            &classification,
            "node",
            FusionTier::Edge,
        );

        let obs = message_to_observation(&msg, Utc::now());
        let original_cov = tracker.variant.covariance_mat();

        // The round-tripped covariance should match the original
        for i in 0..6 {
            for j in 0..6 {
                assert_relative_eq!(
                    obs.covariance.matrix[(i, j)],
                    original_cov[(i, j)],
                    epsilon = 1e-10
                );
            }
        }
    }

    #[test]
    fn state_vector_round_trip() {
        let (track, tracker, quality, classification) = make_test_track();
        let msg = track_to_message(
            &track,
            &tracker,
            &quality,
            &classification,
            "node",
            FusionTier::Regional,
        );

        let original_state = tracker.variant.state_vec();
        let obs = message_to_observation(&msg, Utc::now());

        if let Measurement::FusedEstimate { state, .. } = &obs.measurement {
            for i in 0..6 {
                assert_relative_eq!(state[i], original_state[i], epsilon = 1e-10);
            }
        } else {
            panic!("Expected FusedEstimate measurement");
        }
    }

    #[test]
    fn all_target_categories_round_trip() {
        let categories = vec![
            TargetCategory::FixedWing,
            TargetCategory::Drone,
            TargetCategory::Missile,
            TargetCategory::SurfaceVessel,
            TargetCategory::Submarine,
            TargetCategory::GroundVehicle,
            TargetCategory::Person,
            TargetCategory::Satellite,
            TargetCategory::Unknown,
        ];
        for cat in categories {
            let wire = category_to_wire(cat);
            let back = wire_to_category(wire);
            assert_eq!(cat, back, "Category round-trip failed for {cat:?}");
        }
    }

    #[test]
    fn all_domains_round_trip() {
        let domains = vec![
            TargetDomain::Air,
            TargetDomain::Ground,
            TargetDomain::Maritime,
            TargetDomain::Space,
            TargetDomain::Subsurface,
            TargetDomain::Person,
        ];
        for d in domains {
            let wire = domain_to_wire(d);
            let back = wire_to_domain(wire);
            assert_eq!(d, back, "Domain round-trip failed for {d:?}");
        }
    }

    #[cfg(feature = "nats")]
    #[test]
    fn bincode_serialization_round_trip() {
        let (track, tracker, quality, classification) = make_test_track();
        let msg = track_to_message(
            &track,
            &tracker,
            &quality,
            &classification,
            "bincode-test",
            FusionTier::Regional,
        );

        let bytes = bincode::serialize(&msg).expect("serialize failed");
        assert!(bytes.len() > 0);
        assert!(bytes.len() < 2000); // sanity check: message should be compact

        let decoded: FusedTrackMessage = bincode::deserialize(&bytes).expect("deserialize failed");
        assert_eq!(decoded.node_id, "bincode-test");
        assert_eq!(decoded.state.values.len(), 6);
        assert_eq!(decoded.cooperative_ids[0].id, "ABC123");
    }
}
