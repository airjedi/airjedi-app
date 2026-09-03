//! Sensor kind vocabulary.
//!
//! Relocated from `airjedi-fusion`'s `sensor` module (which now re-exports it)
//! so `SensorContributions` (in `display`) can name the source kind without the
//! fusion engine. The richer `SensorId`/`SensorObservation` types stay in fusion.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
