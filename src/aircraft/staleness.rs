use bevy::prelude::*;
use chrono::Utc;

use crate::adsb::enrichment::PositionSource;
use crate::aircraft::components::FusionDiagnostics;
use crate::theme::AppTheme;
use crate::{Aircraft, AircraftLabel};

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

/// System that dims stale aircraft labels based on time since last ADS-B update.
pub fn dim_stale_aircraft(
    aircraft_query: Query<&Aircraft>,
    mut label_query: Query<(&AircraftLabel, &mut TextColor)>,
) {
    for (label, mut text_color) in label_query.iter_mut() {
        if let Ok(aircraft) = aircraft_query.get(label.aircraft_entity) {
            let elapsed = aircraft_age_secs(aircraft);
            let opacity = staleness_opacity(elapsed);
            text_color.0 = text_color.0.with_alpha(opacity);
        }
    }
}

/// System that colors aircraft label text yellow while tracked via MLAT
/// rather than direct ADS-B, reverting to the theme's default text color
/// otherwise. Preserves whatever alpha `dim_stale_aircraft` has applied.
pub fn color_mlat_labels(
    diag_query: Query<Option<&FusionDiagnostics>, With<Aircraft>>,
    mut label_query: Query<(&AircraftLabel, &mut TextColor)>,
    theme: Res<AppTheme>,
) {
    let mlat_color = Color::srgb(1.0, 1.0, 0.0);

    for (label, mut text_color) in label_query.iter_mut() {
        let is_mlat = diag_query
            .get(label.aircraft_entity)
            .ok()
            .flatten()
            .and_then(|diag| diag.last_position_source)
            .is_some_and(|source| source == PositionSource::Mlat);

        let base = if is_mlat { mlat_color } else { theme.text_primary() };
        text_color.0 = base.with_alpha(text_color.0.alpha());
    }
}
