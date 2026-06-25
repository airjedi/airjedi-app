use bevy::prelude::*;
use std::io::Write;
use std::path::PathBuf;

const OURAIRPORTS_BASE: &str = "https://davidmegginson.github.io/ourairports-data";
const OPENSKY_AIRCRAFT_DB_URL: &str =
    "https://opensky-network.org/datasets/metadata/aircraftDatabase.csv";

/// Aviation data files from various sources
#[derive(Debug, Clone, Copy)]
pub enum DataFile {
    Airports,
    Runways,
    Navaids,
    AircraftDatabase,
}

impl DataFile {
    pub fn filename(&self) -> &'static str {
        match self {
            DataFile::Airports => "airports.csv",
            DataFile::Runways => "runways.csv",
            DataFile::Navaids => "navaids.csv",
            DataFile::AircraftDatabase => "aircraft-database.csv",
        }
    }

    pub fn url(&self) -> String {
        match self {
            DataFile::AircraftDatabase => OPENSKY_AIRCRAFT_DB_URL.to_string(),
            _ => format!("{}/{}", OURAIRPORTS_BASE, self.filename()),
        }
    }
}

/// Download a file from OurAirports (blocking, for use in async task)
pub fn download_file_blocking(file: &DataFile) -> Result<(), String> {
    ensure_cache_dir().map_err(|e| format!("Failed to create cache dir: {}", e))?;

    let url = file.url();
    info!("Downloading {} from {}", file.filename(), url);

    let response = reqwest::blocking::get(&url).map_err(|e| format!("Download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let bytes = response
        .bytes()
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let path = cache_path(file.filename());
    let mut file_handle =
        std::fs::File::create(&path).map_err(|e| format!("Failed to create file: {}", e))?;

    file_handle
        .write_all(&bytes)
        .map_err(|e| format!("Failed to write file: {}", e))?;

    info!("Downloaded {} ({} bytes)", file.filename(), bytes.len());
    Ok(())
}

/// Cache directory for aviation data
fn cache_dir() -> PathBuf {
    crate::paths::cache_dir().join("data")
}

/// Check if cached file exists and is fresh (< 7 days old)
pub fn is_cache_fresh(filename: &str) -> bool {
    let path = cache_dir().join(filename);
    if !path.exists() {
        return false;
    }

    match std::fs::metadata(&path) {
        Ok(meta) => {
            if let Ok(modified) = meta.modified() {
                if let Ok(elapsed) = modified.elapsed() {
                    return elapsed.as_secs() < 7 * 24 * 60 * 60; // 7 days
                }
            }
            false
        }
        Err(_) => false,
    }
}

/// Get the path to a cached file
pub fn cache_path(filename: &str) -> PathBuf {
    cache_dir().join(filename)
}

/// Ensure cache directory exists
pub fn ensure_cache_dir() -> std::io::Result<()> {
    std::fs::create_dir_all(cache_dir())
}
