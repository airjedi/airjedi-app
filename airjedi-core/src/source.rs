//! Position source classification, as reported by readsb's NDJSON `type` field.
//!
//! Relocated from the app's `src/adsb/enrichment.rs` (which now re-exports it).
//! `enrichment.rs` keeps the live NDJSON reader and the ICAO-keyed lookup table;
//! only the enum + its `type`-string mapping live here so `DisplayTrack` can
//! carry a position source across the projection boundary.

use serde::{Deserialize, Serialize};

/// Position source as classified by readsb's `type` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionSource {
    AdsbIcao,
    AdsbIcaoNt,
    AdsrIcao,
    TisbIcao,
    Adsc,
    Mlat,
    Other,
    Unknown,
}

impl PositionSource {
    /// Map readsb's NDJSON `type` string to a position source. An unrecognized
    /// (but present) `type` maps to [`PositionSource::Other`]; a missing `type`
    /// is the caller's concern (see `enrichment.rs`, which uses `Unknown`).
    #[must_use]
    pub fn parse(type_str: &str) -> Self {
        match type_str {
            "adsb_icao" => Self::AdsbIcao,
            "adsb_icao_nt" => Self::AdsbIcaoNt,
            "adsr_icao" => Self::AdsrIcao,
            "tisb_icao" => Self::TisbIcao,
            "adsc" => Self::Adsc,
            "mlat" => Self::Mlat,
            _ => Self::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_types() {
        assert_eq!(PositionSource::parse("adsb_icao"), PositionSource::AdsbIcao);
        assert_eq!(PositionSource::parse("mlat"), PositionSource::Mlat);
        assert_eq!(PositionSource::parse("tisb_icao"), PositionSource::TisbIcao);
    }

    #[test]
    fn unknown_type_becomes_other() {
        assert_eq!(
            PositionSource::parse("some_future_type"),
            PositionSource::Other
        );
    }
}
