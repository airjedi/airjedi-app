//! Track/target identity and domain-classification vocabulary.
//!
//! Relocated verbatim from `airjedi-fusion`'s `types` module (which now
//! re-exports these) so that both the fusion agent and the client can name them
//! without depending on the fusion engine. `Serialize`/`Deserialize` were added
//! for the replication boundary; the `Reflect` derives that were already present
//! are preserved so the app's existing reflection continues to work.

use bevy_reflect::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type Timestamp = chrono::DateTime<chrono::Utc>;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackId(pub Uuid);

impl TrackId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TrackId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetId {
    pub domain: TargetDomain,
    pub id: String,
    pub id_type: IdentifierType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
pub enum TargetDomain {
    Air,
    Ground,
    Maritime,
    Space,
    Subsurface,
    Person,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IdentifierType {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
pub enum TargetCategory {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
pub enum Affiliation {
    Friendly,
    Hostile,
    Neutral,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
