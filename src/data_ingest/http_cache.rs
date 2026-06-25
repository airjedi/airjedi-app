//! HTTP conditional fetch with progressive fallback.
//!
//! Provides a shared `CachedFetcher` that any data provider can use to avoid
//! re-downloading unchanged data. It tries these methods in order:
//!
//! 1. **ETag** (`If-None-Match`) — server returns 304 if content unchanged
//! 2. **Last-Modified** (`If-Modified-Since`) — server returns 304 if not modified
//! 3. **Content-Length** (via HEAD) — skip download if size matches cached file
//! 4. **Full download** — always fetch if nothing else works
//!
//! Per-URL metadata is stored in a `.meta.json` sidecar file alongside the
//! cached data, tracking which methods the server supports so we skip
//! unsupported methods on subsequent requests.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::provider::ProviderError;

/// Result of a conditional fetch operation.
pub enum FetchResult {
    /// Server confirmed data hasn't changed — use cached copy.
    NotModified(Vec<u8>),
    /// New data was downloaded.
    Downloaded(Vec<u8>),
}

impl FetchResult {
    /// Get the data bytes regardless of whether they were cached or fresh.
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            FetchResult::NotModified(b) | FetchResult::Downloaded(b) => b,
        }
    }

    pub fn was_cached(&self) -> bool {
        matches!(self, FetchResult::NotModified(_))
    }
}

/// Metadata stored alongside a cached file, tracking server capabilities
/// and the last known values for conditional request headers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CacheMeta {
    /// Last ETag value from the server (None if server doesn't send ETags).
    etag: Option<String>,
    /// Last Last-Modified value from the server.
    last_modified: Option<String>,
    /// Last known Content-Length.
    content_length: Option<u64>,
    /// Which methods the server is known NOT to support, so we skip them.
    /// Keys: "etag", "last_modified", "content_length"
    #[serde(default)]
    unsupported: HashMap<String, bool>,
}

impl CacheMeta {
    fn supports(&self, method: &str) -> bool {
        !self.unsupported.get(method).copied().unwrap_or(false)
    }

    fn mark_unsupported(&mut self, method: &str) {
        self.unsupported.insert(method.to_string(), true);
    }
}

/// Cache directory for HTTP-cached provider data.
fn cache_dir() -> PathBuf {
    crate::paths::cache_dir().join("data")
}

/// Path to the cached data file for a given cache key.
fn data_path(cache_key: &str) -> PathBuf {
    cache_dir().join(cache_key)
}

/// Path to the metadata sidecar file.
fn meta_path(cache_key: &str) -> PathBuf {
    cache_dir().join(format!("{}.meta.json", cache_key))
}

/// Load cached metadata, or return default if missing/corrupt.
fn load_meta(cache_key: &str) -> CacheMeta {
    let path = meta_path(cache_key);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save metadata to disk.
fn save_meta(cache_key: &str, meta: &CacheMeta) {
    let path = meta_path(cache_key);
    let _ = std::fs::create_dir_all(cache_dir());
    if let Ok(json) = serde_json::to_string_pretty(meta) {
        let _ = std::fs::write(&path, json);
    }
}

/// Fetch a URL with conditional request support and local file caching.
///
/// `cache_key` is the filename used for caching (e.g. "airports.csv").
/// The function progressively tries ETag, Last-Modified, and Content-Length
/// checks, remembering which methods the server supports.
///
/// Returns `FetchResult::NotModified` if the cached copy is still valid,
/// or `FetchResult::Downloaded` with fresh data.
pub fn fetch_with_cache(
    url: &str,
    cache_key: &str,
    timeout_secs: u64,
) -> Result<FetchResult, ProviderError> {
    let cached_path = data_path(cache_key);
    let mut meta = load_meta(cache_key);
    let has_cache = cached_path.exists();

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    // --- Step 1: Try conditional GET with ETag ---
    if has_cache && meta.supports("etag") {
        if let Some(ref etag) = meta.etag {
            let resp = client
                .get(url)
                .header("If-None-Match", etag.as_str())
                .send()
                .map_err(|e| ProviderError::Network(e.to_string()))?;

            if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
                info!("{}: not modified (ETag match)", cache_key);
                let data = read_cache(&cached_path)?;
                return Ok(FetchResult::NotModified(data));
            }

            // If server returned 200 with our If-None-Match, it supports ETags
            // but the content changed. Download the response.
            if resp.status().is_success() {
                return finish_download(resp, cache_key, &cached_path, &mut meta);
            }
        }
    }

    // --- Step 2: Try conditional GET with Last-Modified ---
    if has_cache && meta.supports("last_modified") {
        if let Some(ref last_mod) = meta.last_modified {
            let resp = client
                .get(url)
                .header("If-Modified-Since", last_mod.as_str())
                .send()
                .map_err(|e| ProviderError::Network(e.to_string()))?;

            if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
                info!("{}: not modified (Last-Modified match)", cache_key);
                let data = read_cache(&cached_path)?;
                return Ok(FetchResult::NotModified(data));
            }

            if resp.status().is_success() {
                return finish_download(resp, cache_key, &cached_path, &mut meta);
            }
        }
    }

    // --- Step 3: Try HEAD request to compare Content-Length ---
    if has_cache && meta.supports("content_length") {
        if let Some(cached_len) = meta.content_length {
            let head_resp = client
                .head(url)
                .send()
                .map_err(|e| ProviderError::Network(e.to_string()))?;

            if head_resp.status().is_success() {
                let server_len = head_resp
                    .headers()
                    .get("content-length")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());

                if let Some(len) = server_len {
                    if len == cached_len {
                        info!(
                            "{}: not modified (Content-Length match: {} bytes)",
                            cache_key, len
                        );
                        let data = read_cache(&cached_path)?;
                        return Ok(FetchResult::NotModified(data));
                    }
                    // Size changed — fall through to full download
                } else {
                    // Server doesn't return Content-Length on HEAD
                    meta.mark_unsupported("content_length");
                    save_meta(cache_key, &meta);
                }
            }
        }
    }

    // --- Step 4: Full download ---
    let resp = client
        .get(url)
        .send()
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    if !resp.status().is_success() {
        // Fall back to cache if available
        if has_cache {
            warn!(
                "{}: server returned {}, using stale cache",
                cache_key,
                resp.status()
            );
            let data = read_cache(&cached_path)?;
            return Ok(FetchResult::NotModified(data));
        }
        return Err(ProviderError::Network(format!(
            "{} returned status {}",
            url,
            resp.status()
        )));
    }

    finish_download(resp, cache_key, &cached_path, &mut meta)
}

/// Process a successful response: extract headers, save data and metadata.
fn finish_download(
    resp: reqwest::blocking::Response,
    cache_key: &str,
    cached_path: &Path,
    meta: &mut CacheMeta,
) -> Result<FetchResult, ProviderError> {
    // Extract conditional headers for next time
    let new_etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let new_last_modified = resp
        .headers()
        .get("last-modified")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let content_length = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let bytes = resp
        .bytes()
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    info!("{}: downloaded {} bytes", cache_key, bytes.len());

    // Update metadata with what the server supports
    if let Some(ref etag) = new_etag {
        meta.etag = Some(etag.clone());
    } else {
        meta.mark_unsupported("etag");
    }

    if let Some(ref lm) = new_last_modified {
        meta.last_modified = Some(lm.clone());
    } else {
        meta.mark_unsupported("last_modified");
    }

    meta.content_length = content_length.or(Some(bytes.len() as u64));

    // Save to disk
    let _ = std::fs::create_dir_all(cache_dir());
    if let Ok(mut f) = std::fs::File::create(cached_path) {
        let _ = f.write_all(&bytes);
    }
    save_meta(cache_key, meta);

    Ok(FetchResult::Downloaded(bytes.to_vec()))
}

/// Read cached file from disk.
fn read_cache(path: &Path) -> Result<Vec<u8>, ProviderError> {
    std::fs::read(path).map_err(|e| ProviderError::Other(format!("failed to read cache: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_meta_default_supports_all() {
        let meta = CacheMeta::default();
        assert!(meta.supports("etag"));
        assert!(meta.supports("last_modified"));
        assert!(meta.supports("content_length"));
    }

    #[test]
    fn cache_meta_mark_unsupported() {
        let mut meta = CacheMeta::default();
        meta.mark_unsupported("etag");
        assert!(!meta.supports("etag"));
        assert!(meta.supports("last_modified"));
    }

    #[test]
    fn cache_meta_roundtrip() {
        let mut meta = CacheMeta::default();
        meta.etag = Some("\"abc123\"".to_string());
        meta.last_modified = Some("Thu, 01 Jan 2026 00:00:00 GMT".to_string());
        meta.content_length = Some(12345);
        meta.mark_unsupported("content_length");

        let json = serde_json::to_string(&meta).unwrap();
        let restored: CacheMeta = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.etag, Some("\"abc123\"".to_string()));
        assert_eq!(
            restored.last_modified,
            Some("Thu, 01 Jan 2026 00:00:00 GMT".to_string())
        );
        assert_eq!(restored.content_length, Some(12345));
        assert!(!restored.supports("content_length"));
        assert!(restored.supports("etag"));
    }
}
