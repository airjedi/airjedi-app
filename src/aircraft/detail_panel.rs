use bevy::prelude::*;
use std::time::Instant;

use super::AircraftListState;

/// State for the aircraft detail panel
#[derive(Resource, Default)]
pub struct DetailPanelState {
    pub open: bool,
    /// Timestamp when the selected aircraft was first tracked
    pub track_start: Option<Instant>,
}

/// Resource for camera follow state
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
pub struct CameraFollowState {
    /// ICAO of the aircraft being followed (camera locked to this aircraft)
    pub following_icao: Option<String>,
}

/// Cached data for the detail panel display
pub struct DetailDisplayData {
    pub icao: String,
    pub callsign: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<i32>,
    pub heading: Option<f32>,
    pub velocity: Option<f64>,
    pub vertical_rate: Option<i32>,
    pub distance_nm: f64,
    pub track_points: usize,
    pub track_duration_secs: Option<u64>,
    pub registration: Option<String>,
    pub manufacturer_model: Option<String>,
    pub type_code: Option<String>,
    pub operator: Option<String>,
}

/// Detail panel rendering is now integrated into the stacked right panel
/// (see `render_aircraft_list_panel` in list_panel.rs).
/// This system is kept as a no-op for the plugin registration; the actual
/// rendering happens inside the list panel's bottom section.
pub fn render_detail_panel(
    list_state: Res<AircraftListState>,
    mut detail_state: ResMut<DetailPanelState>,
) {
    // Clear state when no aircraft is selected
    if list_state.selected_icao.is_none() {
        detail_state.open = false;
        detail_state.track_start = None;
    }
}

/// System to toggle detail panel with D key
pub fn toggle_detail_panel(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut detail_state: ResMut<DetailPanelState>,
    list_state: Res<AircraftListState>,
) {
    if keyboard.just_pressed(KeyCode::KeyD) {
        if list_state.selected_icao.is_some() {
            detail_state.open = !detail_state.open;
            if detail_state.open && detail_state.track_start.is_none() {
                detail_state.track_start = Some(Instant::now());
            }
        }
    }
}

/// System to open detail panel when aircraft is selected.
///
/// The bottom detail panel has been replaced by inline expandable cards in the
/// list panel, so this no longer auto-opens the detail pane or forces the list
/// to expand.  It only keeps the track-start timestamp in sync.
pub fn open_detail_on_selection(
    list_state: Res<AircraftListState>,
    mut detail_state: ResMut<DetailPanelState>,
) {
    if !list_state.is_changed() {
        return;
    }

    if list_state.selected_icao.is_some() {
        if detail_state.track_start.is_none() {
            detail_state.track_start = Some(Instant::now());
        }
    } else {
        detail_state.track_start = None;
    }
}

