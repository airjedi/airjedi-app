use bevy::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

/// Raw record from the OpenSky aircraft database CSV.
/// All fields must match CSV column names for serde deserialization.
#[derive(Debug, Deserialize)]
#[allow(dead_code, non_snake_case)]
pub struct AircraftTypeRecord {
    pub icao24: String,
    #[serde(default)]
    pub registration: String,
    #[serde(default)]
    pub manufacturericao: String,
    #[serde(default)]
    pub manufacturername: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub typecode: String,
    #[serde(default)]
    pub serialnumber: String,
    #[serde(default)]
    pub linenumber: String,
    #[serde(default)]
    pub icaoaircrafttype: String,
    #[serde(default)]
    pub operator: String,
    #[serde(default)]
    pub operatorcallsign: String,
    #[serde(default)]
    pub operatoricao: String,
    #[serde(default)]
    pub operatoriata: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub testreg: String,
    #[serde(default)]
    pub registered: String,
    #[serde(default)]
    pub reguntil: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub built: String,
    #[serde(default)]
    pub firstflightdate: String,
    #[serde(default)]
    pub seatconfiguration: String,
    #[serde(default)]
    pub engines: String,
    #[serde(default)]
    pub modes: String,
    #[serde(default)]
    pub adsb: String,
    #[serde(default)]
    pub acars: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub categoryDescription: String,
}

/// Component attached to aircraft entities with resolved type information
#[derive(Component, Debug, Clone)]
pub struct AircraftTypeInfo {
    pub registration: Option<String>,
    pub type_code: Option<String>,
    pub manufacturer_model: Option<String>,
    pub operator: Option<String>,
}

/// Loading state for the aircraft type database
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum LoadingState {
    #[default]
    NotStarted,
    Downloading,
    Ready,
    Failed,
}

/// Resource holding the aircraft type database keyed by lowercase ICAO24
#[derive(Resource, Default)]
pub struct AircraftTypeDatabase {
    pub records: HashMap<String, AircraftTypeRecord>,
    pub loading_state: LoadingState,
}

impl AircraftTypeDatabase {
    /// Look up an ICAO24 address and return display-ready type info
    pub fn lookup(&self, icao: &str) -> Option<AircraftTypeInfo> {
        let record = self.records.get(&icao.to_lowercase())?;

        let registration = non_empty(&record.registration);
        let type_code = non_empty(&record.typecode);

        let manufacturer_model = match (
            non_empty(&record.manufacturername),
            non_empty(&record.model),
        ) {
            (Some(mfr), Some(model)) => Some(format!("{} {}", mfr, model)),
            (None, Some(model)) => Some(model),
            (Some(mfr), None) => Some(mfr),
            (None, None) => None,
        };

        let operator = non_empty(&record.operator);

        Some(AircraftTypeInfo {
            registration,
            type_code,
            manufacturer_model,
            operator,
        })
    }
}

fn non_empty(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
