use chrono::Utc;

use crate::Aircraft;

/// Seconds before an aircraft starts dimming
const STALE_START_SECS: f32 = 10.0;
/// Seconds at which an aircraft is fully dimmed
const STALE_FULL_SECS: f32 = 30.0;
/// Minimum opacity for stale aircraft (low for testing visibility)
const STALE_MIN_OPACITY: f32 = 0.1;

/// Calculate the staleness opacity for an aircraft based on time since last update.
/// Returns 1.0 for fresh aircraft, linearly interpolates to STALE_MIN_OPACITY
/// between STALE_START_SECS and STALE_FULL_SECS.
pub fn staleness_opacity(elapsed_secs: f32) -> f32 {
    if elapsed_secs < STALE_START_SECS {
        1.0
    } else if elapsed_secs < STALE_FULL_SECS {
        let t = (elapsed_secs - STALE_START_SECS) / (STALE_FULL_SECS - STALE_START_SECS);
        1.0 - t * (1.0 - STALE_MIN_OPACITY)
    } else {
        STALE_MIN_OPACITY
    }
}

/// Seconds elapsed since the aircraft's last ADS-B message.
pub fn aircraft_age_secs(aircraft: &Aircraft) -> f32 {
    let now = Utc::now();
    (now - aircraft.last_seen).num_milliseconds().max(0) as f32 / 1000.0
}

/// Format seconds-since-last-message as a compact "Ns" / "Mm:SSs" string
/// for the "last seen" timer shown in the aircraft list and hover popup.
pub fn format_last_seen(elapsed_secs: f32) -> String {
    let secs = elapsed_secs.max(0.0) as u64;
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}
