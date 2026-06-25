pub mod metar;

pub use metar::*;

use bevy::prelude::*;

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WeatherState>()
            .init_resource::<MetarCache>()
            .init_resource::<MetarFetchResults>()
            .add_systems(
                Update,
                (
                    fetch_metar_for_visible_airports,
                    render_weather_indicators,
                    update_weather_indicator_positions,
                    toggle_weather_overlay,
                ),
            );
    }
}
