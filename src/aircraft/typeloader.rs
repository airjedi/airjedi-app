use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::components::Aircraft;
use super::typeinfo::{AircraftTypeDatabase, AircraftTypeInfo, AircraftTypeRecord, LoadingState};
use crate::data::{cache_path, download_file_blocking, is_cache_fresh, DataFile};

/// Resource holding the shared handle to the background loading thread result.
#[derive(Resource)]
pub(crate) struct AircraftTypeLoadHandle(
    Arc<Mutex<Option<Result<HashMap<String, AircraftTypeRecord>, String>>>>,
);

/// Load aircraft type records from the cached CSV
fn load_aircraft_database() -> Result<HashMap<String, AircraftTypeRecord>, String> {
    let path = cache_path(DataFile::AircraftDatabase.filename());
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(&path)
        .map_err(|e| format!("Failed to open aircraft-database.csv: {}", e))?;

    let mut records = HashMap::new();
    for result in rdr.deserialize() {
        match result {
            Ok(record) => {
                let record: AircraftTypeRecord = record;
                let key = record.icao24.to_lowercase();
                if !key.is_empty() {
                    records.insert(key, record);
                }
            }
            Err(e) => {
                // Skip malformed rows silently - the CSV has many inconsistencies
                trace!("Skipping aircraft db row: {}", e);
            }
        }
    }
    Ok(records)
}

/// Startup system: spawns a background thread to download and parse the aircraft type database.
pub fn start_aircraft_type_loading(mut commands: Commands, mut db: ResMut<AircraftTypeDatabase>) {
    if db.loading_state != LoadingState::NotStarted {
        return;
    }

    db.loading_state = LoadingState::Downloading;
    info!("Starting aircraft type database loading in background thread...");

    let result_handle: Arc<Mutex<Option<Result<HashMap<String, AircraftTypeRecord>, String>>>> =
        Arc::new(Mutex::new(None));
    let handle = result_handle.clone();

    std::thread::spawn(move || {
        let file = DataFile::AircraftDatabase;
        if !is_cache_fresh(file.filename()) {
            if let Err(e) = download_file_blocking(&file) {
                let Ok(mut lock) = handle.lock() else {
                    error!("Failed to acquire lock for aircraft type loading");
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

        let result = load_aircraft_database();

        let Ok(mut lock) = handle.lock() else {
            error!("Failed to acquire lock for aircraft type result");
            return;
        };
        *lock = Some(result);
    });

    commands.insert_resource(AircraftTypeLoadHandle(result_handle));
}

/// Update system: polls the background thread and moves data into the resource when ready.
pub fn poll_aircraft_type_loading(
    mut db: ResMut<AircraftTypeDatabase>,
    load_handle: Option<Res<AircraftTypeLoadHandle>>,
) {
    if db.loading_state != LoadingState::Downloading {
        return;
    }

    let Some(handle) = load_handle else {
        return;
    };

    let Ok(mut lock) = handle.0.lock() else {
        error!("Failed to acquire lock for aircraft type poll");
        return;
    };
    let Some(result) = lock.take() else {
        return;
    };

    match result {
        Ok(records) => {
            let count = records.len();
            db.records = records;
            db.loading_state = LoadingState::Ready;
            info!("Aircraft type database ready: {} entries", count);
        }
        Err(e) => {
            error!("Failed to load aircraft type database: {}", e);
            db.loading_state = LoadingState::Failed;
        }
    }
}

/// Update system: attaches AircraftTypeInfo components to aircraft entities that don't have one yet.
pub fn attach_aircraft_type_info(
    mut commands: Commands,
    db: Res<AircraftTypeDatabase>,
    query: Query<(Entity, &Aircraft), Without<AircraftTypeInfo>>,
) {
    if db.loading_state != LoadingState::Ready {
        return;
    }

    for (entity, aircraft) in query.iter() {
        if let Some(type_info) = db.lookup(&aircraft.icao) {
            commands.entity(entity).insert(type_info);
        }
    }
}
