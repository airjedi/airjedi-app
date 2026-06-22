use nalgebra::{DMatrix, DVector};
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
        state: DVector<f64>,
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
    pub is_on_ground: Option<bool>,
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

    #[test]
    fn create_adsb_observation() {
        let obs = SensorObservation {
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
                ..Default::default()
            },
        };
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
            target_id: None,
            measurement: Measurement::Spherical {
                range_m: 50_000.0,
                azimuth_rad: 0.785,
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

    #[test]
    fn create_ais_surface_observation() {
        let obs = SensorObservation {
            sensor_id: SensorId {
                id: "ais-coastal".to_string(),
                kind: SensorKind::AisReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: Some(TargetId {
                domain: TargetDomain::Maritime,
                id: "211234567".to_string(),
                id_type: IdentifierType::Mmsi,
            }),
            measurement: Measurement::PositionVelocity2D {
                lat_deg: 36.85,
                lon_deg: -75.98,
                speed_over_ground_mps: Some(7.7),
                course_over_ground_deg: Some(45.0),
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(4, 4) * 50.0,
            },
            classification_hint: Some(TargetCategory::SurfaceVessel),
            metadata: ObservationMetadata::default(),
        };
        assert_eq!(obs.sensor_id.kind, SensorKind::AisReceiver);
        assert_eq!(
            obs.target_id.as_ref().unwrap().domain,
            TargetDomain::Maritime
        );
    }
}
