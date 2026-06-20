use bevy::prelude::*;
use bevy_slippy_tiles::*;

use crate::{Aircraft, MapState};
use crate::geo::CoordinateConverter;

/// Emergency squawk codes
pub const SQUAWK_HIJACK: &str = "7500";      // Aircraft hijacking
pub const SQUAWK_RADIO_FAIL: &str = "7600";  // Radio failure
pub const SQUAWK_EMERGENCY: &str = "7700";   // General emergency

/// Resource to track active emergency alerts
#[derive(Resource, Default)]
pub struct EmergencyAlertState {
    /// List of aircraft ICAOs currently in emergency
    pub active_emergencies: Vec<EmergencyInfo>,
    /// Animation timer for pulsing effect
    pub pulse_timer: f32,
}

/// Information about an emergency aircraft
#[derive(Clone, Debug)]
pub struct EmergencyInfo {
    pub icao: String,
    pub callsign: Option<String>,
    pub squawk: String,
    pub emergency_type: EmergencyType,
}

/// Type of emergency based on squawk code
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmergencyType {
    Hijack,     // 7500
    RadioFail,  // 7600
    General,    // 7700
}

impl EmergencyType {
    pub fn from_squawk(squawk: &str) -> Option<Self> {
        match squawk {
            SQUAWK_HIJACK => Some(EmergencyType::Hijack),
            SQUAWK_RADIO_FAIL => Some(EmergencyType::RadioFail),
            SQUAWK_EMERGENCY => Some(EmergencyType::General),
            _ => None,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            EmergencyType::Hijack => "HIJACK",
            EmergencyType::RadioFail => "RADIO FAILURE",
            EmergencyType::General => "EMERGENCY",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            EmergencyType::Hijack => Color::srgb(1.0, 0.0, 0.0),     // Bright red
            EmergencyType::RadioFail => Color::srgb(1.0, 0.5, 0.0),  // Orange
            EmergencyType::General => Color::srgb(1.0, 0.2, 0.2),    // Red
        }
    }
}

/// System to detect and track emergency aircraft
pub fn detect_emergencies(
    mut alert_state: ResMut<EmergencyAlertState>,
    aircraft_query: Query<&Aircraft>,
    time: Res<Time>,
) {
    // Update pulse timer
    alert_state.pulse_timer += time.delta_secs() * 3.0; // 3Hz pulse
    if alert_state.pulse_timer > std::f32::consts::TAU {
        alert_state.pulse_timer -= std::f32::consts::TAU;
    }

    // Clear and rebuild emergency list
    alert_state.active_emergencies.clear();

    for aircraft in aircraft_query.iter() {
        if let Some(ref squawk) = aircraft.squawk {
            if let Some(emergency_type) = EmergencyType::from_squawk(squawk) {
                alert_state.active_emergencies.push(EmergencyInfo {
                    icao: aircraft.icao.clone(),
                    callsign: aircraft.callsign.clone(),
                    squawk: squawk.clone(),
                    emergency_type,
                });
            }
        }
    }
}

/// System to draw flashing rings around emergency aircraft
pub fn draw_emergency_rings(
    mut gizmos: Gizmos,
    alert_state: Res<EmergencyAlertState>,
    map_state: Res<MapState>,
    tile_settings: Res<SlippyTilesSettings>,
    view3d_state: Res<crate::view3d::View3DState>,
    aircraft_query: Query<&Aircraft>,
) {
    if alert_state.active_emergencies.is_empty() {
        return;
    }

    // Calculate pulse alpha (0.3 to 1.0 range)
    let pulse = (alert_state.pulse_timer.sin() + 1.0) / 2.0;
    let alpha = 0.3 + pulse * 0.7;

    let zoom = view3d_state.effective_zoom(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, zoom);

    for emergency in &alert_state.active_emergencies {
        // Find the aircraft
        let Some(aircraft) = aircraft_query.iter().find(|a| a.icao == emergency.icao) else {
            continue;
        };

        let pos = converter.latlon_to_world(aircraft.latitude, aircraft.longitude);

        // Get base color for this emergency type
        let base_color = emergency.emergency_type.color();

        // Create pulsing color with alpha
        let color = base_color.with_alpha(alpha);

        // Draw multiple concentric rings with pulsing effect
        let inner_radius = 25.0 + pulse * 5.0;
        let outer_radius = 35.0 + pulse * 10.0;

        gizmos.circle_2d(pos, inner_radius, color);
        gizmos.circle_2d(pos, outer_radius, color.with_alpha(alpha * 0.5));
    }
}

/// Component for the emergency alert banner UI
#[derive(Component)]
pub struct EmergencyBanner;

/// System to show/hide emergency alert banner
pub fn update_emergency_banner(
    mut commands: Commands,
    alert_state: Res<EmergencyAlertState>,
    existing_banner: Query<Entity, With<EmergencyBanner>>,
    theme: Res<crate::theme::AppTheme>,
) {
    let has_emergencies = !alert_state.active_emergencies.is_empty();

    // Remove existing banner if no emergencies
    if !has_emergencies {
        for entity in existing_banner.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }

    // If banner already exists, update it; otherwise create it
    if !existing_banner.is_empty() {
        return; // Banner exists, text update handled by separate system
    }

    // Create emergency banner at top of screen
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(40.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(-200.0)), // Center the 400px wide banner
            width: Val::Px(400.0),
            padding: UiRect::all(Val::Px(10.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(theme.text_error().with_alpha(0.9)),
        EmergencyBanner,
    )).with_children(|parent| {
        parent.spawn((
            Text::new("EMERGENCY ALERT"),
            TextFont {
                font_size: 18.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
    });
}

/// System to update emergency banner text with current emergencies
pub fn update_emergency_banner_text(
    alert_state: Res<EmergencyAlertState>,
    mut banner_query: Query<&Children, With<EmergencyBanner>>,
    mut text_query: Query<&mut Text>,
) {
    if alert_state.active_emergencies.is_empty() {
        return;
    }

    for children in banner_query.iter_mut() {
        for child in children.iter() {
            if let Ok(mut text) = text_query.get_mut(child) {
                // Build emergency text
                let emergency_text: Vec<String> = alert_state.active_emergencies
                    .iter()
                    .map(|e| {
                        let callsign = e.callsign.as_deref().unwrap_or(&e.icao);
                        format!("{}: {} ({})", e.emergency_type.description(), callsign, e.squawk)
                    })
                    .collect();

                **text = emergency_text.join(" | ");
            }
        }
    }
}
