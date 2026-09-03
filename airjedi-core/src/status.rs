//! Track lifecycle status.
//!
//! Relocated from `airjedi-fusion`'s `track` module (which now re-exports it).
//! The transition logic that drives it stays in the fusion engine (`TrackQuality`
//! in `airjedi-fusion`); only the status vocabulary lives here so the client can
//! render it without the engine.

use bevy_reflect::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub enum TrackStatus {
    Tentative,
    Confirmed,
    Coasting,
    Lost,
}
