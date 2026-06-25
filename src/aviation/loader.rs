use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::types::{Airport, Navaid, Runway};
use crate::data::{cache_path, download_file_blocking, is_cache_fresh, DataFile};

/// Resource containing all aviation data
#[derive(Resource, Default)]
pub struct AviationData {
    pub airports: Vec<Airport>,
    pub runways: Vec<Runway>,
    pub navaids: Vec<Navaid>,
    /// Runways indexed by airport_ref for fast lookup
    pub runways_by_airport: HashMap<i64, Vec<usize>>,
    pub loading_state: LoadingState,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum LoadingState {
    #[default]
    NotStarted,
    Downloading,
    Parsing,
    Ready,
    Failed,
}

/// Internal result type from the background loading thread.
struct LoadedData {
    airports: Vec<Airport>,
    runways: Vec<Runway>,
    navaids: Vec<Navaid>,
}

/// Resource holding the shared handle to the background loading thread result.
#[derive(Resource)]
pub struct AviationLoadHandle(Arc<Mutex<Option<Result<LoadedData, String>>>>);

impl AviationData {
    /// Build runway index after loading
    pub fn build_runway_index(&mut self) {
        self.runways_by_airport.clear();
        for (idx, runway) in self.runways.iter().enumerate() {
            self.runways_by_airport
                .entry(runway.airport_ref)
                .or_default()
                .push(idx);
        }
    }

    /// Get runways for an airport
    pub fn get_runways_for_airport(&self, airport_id: i64) -> Vec<&Runway> {
        self.runways_by_airport
            .get(&airport_id)
            .map(|indices| indices.iter().map(|&i| &self.runways[i]).collect())
            .unwrap_or_default()
    }
}

/// Load airports from cached CSV
fn load_airports() -> Result<Vec<Airport>, String> {
    let path = cache_path(DataFile::Airports.filename());
    let mut rdr =
        csv::Reader::from_path(&path).map_err(|e| format!("Failed to open airports.csv: {}", e))?;

    let mut airports = Vec::new();
    for result in rdr.deserialize() {
        match result {
            Ok(airport) => airports.push(airport),
            Err(e) => {
                // Log but continue - some rows may have parsing issues
                warn!("Skipping airport row: {}", e);
            }
        }
    }
    Ok(airports)
}

/// Load runways from cached CSV
fn load_runways() -> Result<Vec<Runway>, String> {
    let path = cache_path(DataFile::Runways.filename());
    let mut rdr =
        csv::Reader::from_path(&path).map_err(|e| format!("Failed to open runways.csv: {}", e))?;

    let mut runways = Vec::new();
    for result in rdr.deserialize() {
        match result {
            Ok(runway) => runways.push(runway),
            Err(e) => {
                warn!("Skipping runway row: {}", e);
            }
        }
    }
    Ok(runways)
}

/// Load navaids from cached CSV
fn load_navaids() -> Result<Vec<Navaid>, String> {
    let path = cache_path(DataFile::Navaids.filename());
    let mut rdr =
        csv::Reader::from_path(&path).map_err(|e| format!("Failed to open navaids.csv: {}", e))?;

    let mut navaids = Vec::new();
    for result in rdr.deserialize() {
        match result {
            Ok(navaid) => navaids.push(navaid),
            Err(e) => {
                warn!("Skipping navaid row: {}", e);
            }
        }
    }
    Ok(navaids)
}

/// Startup system: spawns a background thread to download and parse aviation data.
pub fn start_aviation_data_loading(
    mut commands: Commands,
    mut aviation_data: ResMut<AviationData>,
) {
    if aviation_data.loading_state != LoadingState::NotStarted {
        return;
    }

    aviation_data.loading_state = LoadingState::Downloading;
    info!("Starting aviation data loading in background thread...");

    let result_handle: Arc<Mutex<Option<Result<LoadedData, String>>>> = Arc::new(Mutex::new(None));
    let handle = result_handle.clone();

    std::thread::spawn(move || {
        // Download phase
        let files = [DataFile::Airports, DataFile::Runways, DataFile::Navaids];
        for file in &files {
            if !is_cache_fresh(file.filename()) {
                if let Err(e) = download_file_blocking(file) {
                    let Ok(mut lock) = handle.lock() else {
                        error!("Failed to acquire lock for aviation data loading");
                        return;
                    };
                    *lock = Some(Err(format!(
                        "Failed to download {}: {}",
                        file.filename(),
                        e
                    )));
                    return;
                }
            }
        }

        // Parse phase
        let result = match (load_airports(), load_runways(), load_navaids()) {
            (Ok(airports), Ok(runways), Ok(navaids)) => Ok(LoadedData {
                airports,
                runways,
                navaids,
            }),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
        };

        let Ok(mut lock) = handle.lock() else {
            error!("Failed to acquire lock for aviation data result");
            return;
        };
        *lock = Some(result);
    });

    commands.insert_resource(AviationLoadHandle(result_handle));
}

/// Update system: polls the background thread and moves data into the ECS
/// resource when loading is complete.
pub fn poll_aviation_data_loading(
    mut aviation_data: ResMut<AviationData>,
    load_handle: Option<Res<AviationLoadHandle>>,
) {
    // Only poll while we're in the loading states
    if !matches!(
        aviation_data.loading_state,
        LoadingState::Downloading | LoadingState::Parsing
    ) {
        return;
    }

    let Some(handle) = load_handle else {
        return;
    };

    let Ok(mut lock) = handle.0.lock() else {
        error!("Failed to acquire lock for aviation data poll");
        return;
    };
    let Some(result) = lock.take() else {
        // Still loading
        return;
    };

    match result {
        Ok(data) => {
            let airport_count = data.airports.len();
            let runway_count = data.runways.len();
            let navaid_count = data.navaids.len();

            aviation_data.airports = data.airports;
            aviation_data.runways = data.runways;
            aviation_data.navaids = data.navaids;
            aviation_data.build_runway_index();
            aviation_data.loading_state = LoadingState::Ready;
            info!(
                "Aviation data ready: {} airports, {} runways, {} navaids",
                airport_count, runway_count, navaid_count
            );
        }
        Err(e) => {
            error!("Failed to load aviation data: {}", e);
            aviation_data.loading_state = LoadingState::Failed;
        }
    }
}
