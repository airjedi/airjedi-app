mod player;
mod recorder;

pub use player::*;
pub use recorder::*;

use bevy::prelude::*;

pub struct RecordingPlugin;

impl Plugin for RecordingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RecordingState>()
            .init_resource::<PlaybackState>()
            .add_systems(Update, (record_frame, playback_frame, toggle_recording));
    }
}
