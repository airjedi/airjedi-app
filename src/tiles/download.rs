use async_lock::Semaphore;
use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, IoTaskPool, Task};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::types::*;
use super::coords::SlippyTileCoordinates;

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Clone, Resource)]
pub struct TileDownloadSettings {
    pub endpoint: String,
    pub tiles_directory: PathBuf,
    pub max_concurrent_downloads: usize,
    pub max_retries: u32,
    pub tile_format: TileFormat,
    pub tile_size: TileSize,
    pub rate_limit_requests: usize,
    pub rate_limit_window: Duration,
    /// When true, swap x and y in the tile URL (/{z}/{y}/{x} servers).
    pub reverse_axes: bool,
    /// Whether this provider supports @2x retina postfix in URLs.
    pub supports_retina: bool,
    /// Whether this provider uses file extensions in tile URLs.
    pub uses_extension_in_url: bool,
    /// Basemap style key for per-style cache directories (e.g. "carto-dark").
    pub cache_key: String,
}

impl Default for TileDownloadSettings {
    fn default() -> Self {
        Self {
            endpoint: "https://tile.openstreetmap.org".into(),
            tiles_directory: PathBuf::from("tiles/"),
            max_concurrent_downloads: 16,
            max_retries: 3,
            tile_format: TileFormat::default(),
            tile_size: TileSize::Large,
            rate_limit_requests: 60,
            rate_limit_window: Duration::from_secs(1),
            reverse_axes: false,
            supports_retina: true,
            uses_extension_in_url: true,
            cache_key: "carto-dark".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Request to download tiles around a location.
#[derive(Message)]
pub struct DownloadTilesRequest {
    pub latitude: f64,
    pub longitude: f64,
    pub zoom: u8,
    pub radius: Radius,
    pub priority: DownloadPriority,
    pub use_cache: bool,
}

/// Fired when a tile has been downloaded (or loaded from cache) and is ready.
#[derive(Message, Clone)]
pub struct TileReady {
    pub key: TileKey,
    pub path: PathBuf,
    pub from_cache: bool,
}

// ---------------------------------------------------------------------------
// Download task result
// ---------------------------------------------------------------------------

struct TileDownloadResult {
    key: TileKey,
    path: PathBuf,
    success: bool,
    from_cache: bool,
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

#[derive(Resource)]
struct DownloadSemaphore(Arc<Semaphore>);

#[derive(Resource, Default)]
struct ActiveDownloads(HashMap<TileKey, Task<TileDownloadResult>>);

#[derive(Resource, Default)]
pub struct DownloadedTiles(HashSet<TileKey>);

#[derive(Resource, Default)]
struct FailedTileCooldown {
    failures: HashMap<TileKey, Instant>,
}

impl FailedTileCooldown {
    const COOLDOWN: Duration = Duration::from_secs(30);

    fn record_failure(&mut self, key: TileKey) {
        self.failures.insert(key, Instant::now());
    }

    fn is_cooling_down(&self, key: &TileKey) -> bool {
        self.failures
            .get(key)
            .map(|t| t.elapsed() < Self::COOLDOWN)
            .unwrap_or(false)
    }

    fn prune_expired(&mut self) {
        self.failures.retain(|_, t| t.elapsed() < Self::COOLDOWN);
    }
}

struct BufferedRequest {
    key: TileKey,
    endpoint: String,
    filename: String,
    priority: DownloadPriority,
}

#[derive(Resource, Default)]
struct RateLimiter {
    timestamps: VecDeque<Instant>,
    buffer: Vec<BufferedRequest>,
}

impl RateLimiter {
    fn can_request(&mut self, now: Instant, window: Duration, limit: usize) -> bool {
        while self.timestamps.front().map_or(false, |t| now.duration_since(*t) > window) {
            self.timestamps.pop_front();
        }
        self.timestamps.len() < limit
    }

    fn clear_stale_zoom(&mut self, current_zoom: u8) {
        let before = self.buffer.len();
        self.buffer.retain(|r| r.key.zoom == current_zoom);
        let removed = before - self.buffer.len();
        if removed > 0 {
            debug!("Cleared {} stale buffered tile requests", removed);
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin setup
// ---------------------------------------------------------------------------

pub(super) fn setup_download_systems(app: &mut App) {
    app.init_resource::<ActiveDownloads>()
        .init_resource::<DownloadedTiles>()
        .init_resource::<FailedTileCooldown>()
        .init_resource::<RateLimiter>()
        .add_message::<DownloadTilesRequest>()
        .add_message::<TileReady>()
        .add_systems(Startup, init_semaphore)
        .add_systems(
            Update,
            (process_download_requests, poll_active_downloads).chain(),
        );
}

fn init_semaphore(mut commands: Commands, settings: Res<TileDownloadSettings>) {
    commands.insert_resource(DownloadSemaphore(Arc::new(Semaphore::new(
        settings.max_concurrent_downloads,
    ))));
}

// ---------------------------------------------------------------------------
// Core download systems
// ---------------------------------------------------------------------------

fn process_download_requests(
    mut requests: MessageReader<DownloadTilesRequest>,
    settings: Res<TileDownloadSettings>,
    mut active: ResMut<ActiveDownloads>,
    mut downloaded: ResMut<DownloadedTiles>,
    mut cooldown: ResMut<FailedTileCooldown>,
    mut rate_limiter: ResMut<RateLimiter>,
    semaphore: Res<DownloadSemaphore>,
    mut ready_writer: MessageWriter<TileReady>,
) {
    let now = Instant::now();

    // Periodically prune expired cooldowns
    cooldown.prune_expired();

    // Process buffered requests first (priority-sorted)
    rate_limiter.buffer.sort_by_key(|r| r.priority);
    let mut i = 0;
    while i < rate_limiter.buffer.len() {
        if !rate_limiter.can_request(now, settings.rate_limit_window, settings.rate_limit_requests) {
            break;
        }
        let req = rate_limiter.buffer.remove(i);
        spawn_download(
            req.key,
            req.endpoint,
            req.filename,
            &settings,
            &semaphore,
            &mut active,
        );
        rate_limiter.timestamps.push_back(now);
    }

    // Process new requests
    for request in requests.read() {
        let Ok(zoom_level) = super::types::ZoomLevel::try_from(request.zoom) else {
            warn!("Invalid zoom level: {}", request.zoom);
            continue;
        };
        let center = SlippyTileCoordinates::from_latitude_longitude(
            request.latitude,
            request.longitude,
            zoom_level,
        );

        rate_limiter.clear_stale_zoom(request.zoom);

        let radius = request.radius.0 as i64;
        let max_tile = 1i64 << request.zoom;

        for dx in -radius..=radius {
            for dy in -radius..=radius {
                let raw_x = center.x as i64 + dx;
                let y = center.y as i64 + dy;

                if y < 0 || y >= max_tile {
                    continue;
                }

                let x = super::coords::wrap_tile_x(raw_x, request.zoom);

                let key = TileKey {
                    x,
                    y: y as u32,
                    zoom: request.zoom,
                    tile_size: settings.tile_size,
                    tile_format: settings.tile_format,
                };

                if active.0.contains_key(&key) || cooldown.is_cooling_down(&key) {
                    continue;
                }

                let filename = tile_filename(&settings, &key);
                let file_path = tile_cache_path(&settings, &key);

                // Check per-style cache first, then fall back to flat cache
                // (tiles cached before per-style directories were added).
                let cached = if file_path.exists() {
                    true
                } else {
                    let flat_path = tile_cache_path_flat(&key);
                    if flat_path.exists() {
                        // Migrate to per-style directory for future lookups
                        if let Some(parent) = file_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::copy(&flat_path, &file_path);
                        true
                    } else {
                        false
                    }
                };

                if request.use_cache && cached {
                    ready_writer.write(TileReady {
                        key,
                        path: PathBuf::from(&filename),
                        from_cache: true,
                    });
                    continue;
                }

                // Skip if already downloading
                if downloaded.0.contains(&key) {
                    continue;
                }

                let endpoint = settings.endpoint.clone();
                rate_limiter.buffer.push(BufferedRequest {
                    key,
                    endpoint,
                    filename,
                    priority: request.priority,
                });
            }
        }

        // Try to send buffered requests
        rate_limiter.buffer.sort_by_key(|r| r.priority);
        let mut j = 0;
        while j < rate_limiter.buffer.len() {
            if !rate_limiter.can_request(now, settings.rate_limit_window, settings.rate_limit_requests)
            {
                break;
            }
            let req = rate_limiter.buffer.remove(j);
            spawn_download(
                req.key,
                req.endpoint,
                req.filename,
                &settings,
                &semaphore,
                &mut active,
            );
            rate_limiter.timestamps.push_back(now);
        }
    }
}

fn poll_active_downloads(
    mut active: ResMut<ActiveDownloads>,
    mut downloaded: ResMut<DownloadedTiles>,
    mut cooldown: ResMut<FailedTileCooldown>,
    mut ready_writer: MessageWriter<TileReady>,
) {
    let mut completed = Vec::new();

    for (key, task) in active.0.iter_mut() {
        if let Some(result) = future::block_on(future::poll_once(task)) {
            completed.push((key.clone(), result));
        }
    }

    for (key, result) in completed {
        active.0.remove(&key);

        if result.success {
            downloaded.0.insert(result.key.clone());
            ready_writer.write(TileReady {
                key: result.key,
                path: result.path,
                from_cache: result.from_cache,
            });
        } else {
            cooldown.record_failure(result.key);
        }
    }
}

// ---------------------------------------------------------------------------
// Download helpers
// ---------------------------------------------------------------------------

fn spawn_download(
    key: TileKey,
    endpoint: String,
    filename: String,
    settings: &TileDownloadSettings,
    semaphore: &DownloadSemaphore,
    active: &mut ActiveDownloads,
) {
    let url = tile_url(&endpoint, &key, settings);
    let accept_mime = key.tile_format.accept_mime().to_string();
    let max_retries = settings.max_retries;
    let sem = Arc::clone(&semaphore.0);
    let cache_path = tile_cache_path(settings, &key);
    let key_clone = key.clone();

    let task = IoTaskPool::get().spawn(async move {
        let mut retries = 0u32;

        loop {
            if retries >= max_retries {
                warn!("Max retries for tile {}", url);
                return TileDownloadResult {
                    key: key_clone,
                    path: PathBuf::from(&filename),
                    success: false,
                    from_cache: false,
                };
            }

            let request = ehttp::Request {
                method: "GET".to_owned(),
                url: url.clone(),
                body: vec![],
                headers: ehttp::Headers::new(&[
                    ("User-Agent", "airjedi/1.0"),
                    ("Accept", &accept_mime),
                ]),
            };

            let result = {
                let _guard = sem.acquire().await;
                ehttp::fetch_async(request).await
            };

            match result {
                Ok(response) if response.status == 200 => {
                    let bytes = &response.bytes;

                    if !validate_tile_bytes(bytes, &key_clone.tile_format) {
                        warn!("Invalid tile content from {}", url);
                        retries += 1;
                        continue;
                    }

                    if let Err(e) = atomic_write(&cache_path, bytes) {
                        warn!("Failed to write tile {}: {}", cache_path.display(), e);
                        retries += 1;
                        continue;
                    }

                    return TileDownloadResult {
                        key: key_clone,
                        path: PathBuf::from(&filename),
                        success: true,
                        from_cache: false,
                    };
                }
                Ok(response) => {
                    warn!("HTTP {} for tile {}", response.status, url);
                    retries += 1;
                }
                Err(e) => {
                    warn!("Download error for {}: {}", url, e);
                    retries += 1;
                }
            }
        }
    });

    active.0.insert(key, task);
}

fn tile_url(endpoint: &str, key: &TileKey, settings: &TileDownloadSettings) -> String {
    let postfix = if settings.supports_retina {
        key.tile_size.url_postfix()
    } else {
        ""
    };

    let (first, second) = if settings.reverse_axes {
        (key.y, key.x)
    } else {
        (key.x, key.y)
    };

    if settings.uses_extension_in_url {
        let ext = key.tile_format.extension();
        format!("{}/{}/{}/{}{}.{}", endpoint, key.zoom, first, second, postfix, ext)
    } else {
        format!("{}/{}/{}/{}", endpoint, key.zoom, first, second)
    }
}

fn tile_filename(settings: &TileDownloadSettings, key: &TileKey) -> String {
    let dir = settings.tiles_directory.to_string_lossy();
    let ext = key.tile_format.extension();
    format!(
        "{}{}/{}.{}.{}.{}.tile.{}",
        dir,
        settings.cache_key,
        key.zoom,
        key.x,
        key.y,
        key.tile_size.to_pixels(),
        ext,
    )
}

fn tile_cache_path(settings: &TileDownloadSettings, key: &TileKey) -> PathBuf {
    let cache_dir = crate::tile_cache::tile_cache_dir_for_style(&settings.cache_key);
    let ext = key.tile_format.extension();
    cache_dir.join(format!(
        "{}.{}.{}.{}.tile.{}",
        key.zoom,
        key.x,
        key.y,
        key.tile_size.to_pixels(),
        ext,
    ))
}

/// Check the old flat cache directory (no style subdirectory).
fn tile_cache_path_flat(key: &TileKey) -> PathBuf {
    let cache_dir = crate::tile_cache::tile_cache_dir();
    let ext = key.tile_format.extension();
    cache_dir.join(format!(
        "{}.{}.{}.{}.tile.{}",
        key.zoom,
        key.x,
        key.y,
        key.tile_size.to_pixels(),
        ext,
    ))
}

fn validate_tile_bytes(bytes: &[u8], format: &TileFormat) -> bool {
    const MIN_TILE_SIZE: usize = 100;
    if bytes.len() < MIN_TILE_SIZE {
        return false;
    }
    // Reject HTML/XML error pages
    if bytes.starts_with(b"<") || bytes.starts_with(b"<!") || bytes.starts_with(b"<?xml") {
        return false;
    }
    match format {
        TileFormat::Png => bytes.len() >= 8 && bytes[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
        TileFormat::Jpg => bytes.len() >= 2 && bytes[..2] == [0xFF, 0xD8],
        TileFormat::Webp => bytes.len() >= 4 && &bytes[..4] == b"RIFF",
    }
}

fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Clear the downloaded tiles tracking set. Used when basemap changes.
pub fn clear_download_tracking(downloaded: &mut DownloadedTiles) {
    downloaded.0.clear();
}

/// Access the DownloadedTiles resource type for external systems.
pub type DownloadedTilesRes = DownloadedTiles;
