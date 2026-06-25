/// Centralized tile cache management.
///
/// Stores map tiles in the platform cache directory (`~/Library/Caches/airjedi/tiles`
/// on macOS) and symlinks them into the Bevy assets directory so the AssetServer
/// can load them transparently.
use bevy::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

/// Returns the platform-appropriate tile cache directory.
///
/// - macOS:   `~/Library/Caches/airjedi/tiles`
/// - Linux:   `~/.cache/airjedi/tiles`
/// - Windows: `%LOCALAPPDATA%\airjedi\cache\tiles`
pub fn tile_cache_dir() -> PathBuf {
    crate::paths::cache_dir().join("tiles")
}

/// Ensures the centralized cache directory exists and is symlinked into
/// `assets/tiles` so bevy_slippy_tiles can read/write through the AssetServer.
///
/// Called once at startup before any tile downloads.
pub fn setup_tile_cache() {
    let cache_dir = tile_cache_dir();

    // Create the cache directory if it doesn't exist
    if let Err(e) = fs::create_dir_all(&cache_dir) {
        warn!(
            "Failed to create tile cache directory {:?}: {}",
            cache_dir, e
        );
        return;
    }

    // Determine the assets/tiles symlink path
    let assets_tiles = assets_tiles_path();

    // If assets/tiles already exists, check if it's correct
    if assets_tiles.exists() || assets_tiles.symlink_metadata().is_ok() {
        if assets_tiles.symlink_metadata().is_ok() {
            if let Ok(target) = fs::read_link(&assets_tiles) {
                if target == cache_dir {
                    info!(
                        "Tile cache symlink already correct: {:?} -> {:?}",
                        assets_tiles, cache_dir
                    );
                    return;
                }
                // Symlink points elsewhere, remove and recreate
                warn!(
                    "Tile cache symlink points to {:?}, updating to {:?}",
                    target, cache_dir
                );
                let _ = fs::remove_file(&assets_tiles);
            } else {
                // It's a real directory, not a symlink — migrate any existing tiles
                migrate_existing_tiles(&assets_tiles, &cache_dir);
            }
        }
    }

    // Ensure the assets directory exists
    let assets_dir = assets_tiles.parent().unwrap_or(Path::new("assets"));
    if let Err(e) = fs::create_dir_all(assets_dir) {
        warn!("Failed to create assets directory {:?}: {}", assets_dir, e);
        return;
    }

    // Create the symlink
    #[cfg(unix)]
    {
        if let Err(e) = std::os::unix::fs::symlink(&cache_dir, &assets_tiles) {
            warn!(
                "Failed to create tile cache symlink {:?} -> {:?}: {}",
                assets_tiles, cache_dir, e
            );
            return;
        }
    }

    #[cfg(windows)]
    {
        if let Err(e) = std::os::windows::fs::symlink_dir(&cache_dir, &assets_tiles) {
            warn!(
                "Failed to create tile cache symlink {:?} -> {:?}: {}",
                assets_tiles, cache_dir, e
            );
            return;
        }
    }

    info!("Tile cache: {:?} -> {:?}", assets_tiles, cache_dir);
}

/// Clears all cached tile files from the centralized cache directory.
pub fn clear_tile_cache() {
    let cache_dir = tile_cache_dir();

    if !cache_dir.exists() {
        warn!("Tile cache directory not found at {:?}", cache_dir);
        return;
    }

    let mut deleted_count = 0;

    if let Ok(entries) = fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    if filename.contains(".tile.") {
                        if let Err(e) = fs::remove_file(&path) {
                            warn!("Failed to delete tile {:?}: {}", path, e);
                        } else {
                            deleted_count += 1;
                        }
                    }
                }
            }
        }
    }

    info!(
        "Cleared {} tile(s) from cache at {:?}",
        deleted_count, cache_dir
    );
}

/// Also clear any legacy tiles sitting directly in `assets/` from before
/// the centralized cache was introduced.
pub fn clear_legacy_tiles() {
    let assets_path = crate::paths::assets_dir();

    if !assets_path.exists() {
        return;
    }

    let mut deleted_count = 0;
    if let Ok(entries) = fs::read_dir(&assets_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    if filename.contains(".tile.") {
                        if let Err(e) = fs::remove_file(&path) {
                            warn!("Failed to delete legacy tile {:?}: {}", path, e);
                        } else {
                            deleted_count += 1;
                        }
                    }
                }
            }
        }
    }

    if deleted_count > 0 {
        info!("Cleared {} legacy tile(s) from assets/", deleted_count);
    }
}

/// Remove cached tiles whose file content doesn't match their extension.
/// Invalid tiles cause Bevy's asset loader to log errors every frame.
pub fn remove_invalid_tiles() {
    let cache_dir = tile_cache_dir();
    if !cache_dir.exists() {
        return;
    }

    let png_signature: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let jpg_signature: [u8; 2] = [0xFF, 0xD8]; // JPEG SOI
    let mut removed = 0;

    if let Ok(entries) = fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // Determine expected format from file extension
            let expected = if filename.ends_with(".tile.png") {
                "png"
            } else if filename.ends_with(".tile.jpg") {
                "jpg"
            } else if filename.ends_with(".tile.webp") {
                "webp"
            } else {
                continue;
            };
            // Read first bytes and validate against expected format
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(_) => {
                    let _ = fs::remove_file(&path);
                    removed += 1;
                    continue;
                }
            };
            let valid = match expected {
                "png" => bytes.len() >= 8 && bytes[..8] == png_signature,
                "jpg" => bytes.len() >= 2 && bytes[..2] == jpg_signature,
                "webp" => bytes.len() >= 4 && &bytes[..4] == b"RIFF",
                _ => true,
            };
            if !valid {
                let hex_header: String = bytes
                    .iter()
                    .take(16)
                    .map(|b| format!("{:02x}", b))
                    .collect::<Vec<_>>()
                    .join(" ");
                let ascii_preview: String = bytes
                    .iter()
                    .take(64)
                    .map(|&b| {
                        if b.is_ascii_graphic() || b == b' ' {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                warn!(
                    "Invalid tile at startup: {} size={} bytes, header=[{}], preview=[{}]",
                    filename,
                    bytes.len(),
                    hex_header,
                    ascii_preview
                );
                let _ = fs::remove_file(&path);
                removed += 1;
            }
        }
    }

    if removed > 0 {
        info!("Removed {} invalid tile(s) from cache", removed);
    }
}

/// Validate a single tile file's header. Returns `true` if the file was corrupt
/// and was deleted, `false` if the file is valid or doesn't exist.
pub fn validate_and_remove_if_corrupt(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => {
            // Can't read the file — remove it
            let _ = fs::remove_file(path);
            return true;
        }
    };

    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    let png_signature: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let jpg_signature: [u8; 2] = [0xFF, 0xD8];

    let valid = if filename.ends_with(".tile.png") || filename.ends_with(".png") {
        bytes.len() >= 8 && bytes[..8] == png_signature
    } else if filename.ends_with(".tile.jpg") || filename.ends_with(".jpg") {
        bytes.len() >= 2 && bytes[..2] == jpg_signature
    } else if filename.ends_with(".tile.webp") || filename.ends_with(".webp") {
        bytes.len() >= 4 && &bytes[..4] == b"RIFF"
    } else {
        // Unknown format, assume valid
        true
    };

    if !valid {
        // Log diagnostic details to help identify the corruption source
        let hex_header: String = bytes
            .iter()
            .take(16)
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ");
        let ascii_preview: String = bytes
            .iter()
            .take(64)
            .map(|&b| {
                if b.is_ascii_graphic() || b == b' ' {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        warn!(
            "Corrupt tile: {:?} size={} bytes, header=[{}], preview=[{}]",
            path,
            bytes.len(),
            hex_header,
            ascii_preview
        );
        let _ = fs::remove_file(path);
        true
    } else {
        false
    }
}

/// Check and remove a corrupt cached tile given its asset path (relative to assets/).
/// Called when Bevy's asset loader fails to load a tile image.
/// The asset path is expected to be like `tiles/10.512.340.256.tile.png`.
pub fn remove_corrupt_cached_tile(asset_path: &Path) {
    // The asset path is relative to the assets/ directory. Since assets/tiles
    // is a symlink to the cache directory, resolve to the actual cache path.
    let cache_dir = tile_cache_dir();
    // Strip the leading "tiles/" component to get just the filename
    let filename = asset_path.file_name().unwrap_or(asset_path.as_os_str());
    let cache_path = cache_dir.join(filename);

    if validate_and_remove_if_corrupt(&cache_path) {
        info!(
            "Removed corrupt cached tile {:?}, will re-download on next request",
            cache_path
        );
    }
}

/// Returns the platform-appropriate terrain/elevation cache directory.
///
/// - macOS:   `~/Library/Caches/airjedi/terrain`
/// - Linux:   `~/.cache/airjedi/terrain`
/// - Windows: `%LOCALAPPDATA%\airjedi\cache\terrain`
pub fn terrain_cache_dir() -> PathBuf {
    crate::paths::cache_dir().join("terrain")
}

/// Ensures the terrain cache directory exists.
/// Called once at startup alongside `setup_tile_cache()`.
pub fn setup_terrain_cache() {
    let cache_dir = terrain_cache_dir();
    if let Err(e) = fs::create_dir_all(&cache_dir) {
        warn!(
            "Failed to create terrain cache directory {:?}: {}",
            cache_dir, e
        );
        return;
    }
    info!("Terrain cache: {:?}", cache_dir);
}

fn assets_tiles_path() -> PathBuf {
    crate::paths::assets_dir().join("tiles")
}

/// Move existing tiles from a real `assets/tiles/` directory into the cache,
/// then remove the directory so it can be replaced with a symlink.
fn migrate_existing_tiles(source: &Path, dest: &Path) {
    info!("Migrating existing tiles from {:?} to {:?}", source, dest);
    let mut migrated = 0;

    if let Ok(entries) = fs::read_dir(source) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name() {
                    let dest_path = dest.join(filename);
                    if let Err(e) = fs::rename(&path, &dest_path) {
                        warn!("Failed to migrate tile {:?}: {}", path, e);
                    } else {
                        migrated += 1;
                    }
                }
            }
        }
    }

    // Remove the now-empty directory
    let _ = fs::remove_dir(source);
    info!("Migrated {} tile(s) to centralized cache", migrated);
}
